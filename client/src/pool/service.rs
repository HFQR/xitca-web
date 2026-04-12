//! pluggable connection pool as a [Service] layer.
//!
//! [PoolService] is a [Service] implementation that receives a [PoolRequest]
//! and produces a [Lease] — a leased connection the caller can use for
//! protocol dispatch. An end user can provide a custom pool implementation via
//! [ClientBuilder::pool] to replace the default behavior.
//!
//! Connection making (dns resolve, tcp / tls connect, alpn negotiation,
//! h2 / h3 handshake, h2c fallback) is exposed through [PoolRequest::spawn]
//! so a user pool can reach the crate-internal connection primitives without
//! any dependency on crate-private modules.
//!
//! [ClientBuilder::pool]: crate::builder::ClientBuilder::pool
//! [Service]: crate::service::Service

use core::{
    ops::{Deref, DerefMut},
    time::Duration,
};

use crate::{
    client::Client,
    connect::Connect,
    connection::{ConnectionExclusive, ConnectionKey, ConnectionShared},
    error::Error,
    http::Version,
    pool::{exclusive, shared},
    service::{Service, ServiceDyn},
};

// -----------------------------------------------------------------------------
// public request / response types
// -----------------------------------------------------------------------------

/// request type for [PoolService]. A pool receives a [PoolRequest] and must
/// produce a [Lease] to either a cached or newly established connection.
///
/// Use [PoolRequest::spawn] on cache-miss to reach the crate-internal
/// connection establishment logic (dns resolve, tcp / tls connect, h2 / h3
/// handshake, version fallback). The produced lease is "plain" and not
/// attached to any cache — managing reuse is the user pool's responsibility.
pub struct PoolRequest<'a, 'c> {
    /// the [Client] the request originated from. pool implementations can use
    /// it to reach resolver / tls connector / timeout configuration.
    pub client: &'a Client,
    /// connection info. dns resolution (if any) will mutate this value.
    pub connect: Connect<'c>,
    /// requested http version. [PoolRequest::spawn] may downgrade this when
    /// alpn negotiates a lower version or an h3 connection cannot be
    /// established; the actually negotiated version is carried on [Lease].
    pub version: Version,
    /// when true the pool is allowed to transparently downgrade a failed
    /// prior-knowledge h2c handshake to http/1. gRPC callers must set this
    /// to false because an http/1 response cannot carry valid gRPC framing.
    pub allow_h2c_downgrade: bool,
}

impl PoolRequest<'_, '_> {
    /// spawn a fresh connection using the crate-internal connection
    /// establishment logic.
    ///
    /// On success, returns a plain [Lease] — a lease whose drop does not
    /// return the connection to any cache. The user pool is expected to
    /// install its own caching / re-use strategy around this primitive.
    ///
    /// Runs the full version negotiation loop: http/3 falls back to http/2
    /// on connect failure, http/2 falls back to http/1 when alpn selects
    /// http/1 or when [allow_h2c_downgrade] is enabled and an h2c
    /// prior-knowledge handshake fails.
    ///
    /// [allow_h2c_downgrade]: PoolRequest::allow_h2c_downgrade
    pub async fn spawn(&mut self) -> Result<Lease, Error> {
        let mut ver = self.version;
        loop {
            match establish(self.client, &mut self.connect, ver, self.allow_h2c_downgrade).await? {
                EstablishOutcome::Exclusive { conn, version } => {
                    return Ok(Lease::Exclusive {
                        conn: ExclusiveLease::new(PlainExclusive::new(conn)),
                        version,
                    });
                }
                #[cfg(any(feature = "http2", feature = "http3"))]
                EstablishOutcome::Shared { conn, version } => {
                    return Ok(Lease::Shared {
                        conn: SharedLease::new(PlainShared::new(conn)),
                        version,
                    });
                }
                EstablishOutcome::RetryLower(lower) => {
                    ver = lower;
                }
            }
        }
    }
}

/// type alias for an object safe [Service] implementation for connection pools.
///
/// [Service]: crate::service::Service
pub type PoolService =
    Box<dyn for<'a, 'c> ServiceDyn<PoolRequest<'a, 'c>, Response = Lease, Error = Error> + Send + Sync>;

/// a leased connection returned by a [PoolService].
pub enum Lease {
    /// an exclusive (http/1) lease.
    Exclusive {
        conn: ExclusiveLease,
        /// negotiated version. always http/1.x for this variant.
        version: Version,
    },
    /// a shared (http/2 or http/3) lease.
    Shared {
        conn: SharedLease,
        /// negotiated version. http/2 or http/3.
        version: Version,
    },
}

/// implementation hook for [ExclusiveLease]. a custom pool implementation
/// provides this trait on its leased connection type.
pub trait ExclusiveLeaseInner: Send + Sync {
    /// immutable access to the underlying connection.
    fn get(&self) -> &ConnectionExclusive;
    /// mutable access to the underlying connection.
    fn get_mut(&mut self) -> &mut ConnectionExclusive;
    /// mark the connection as not re-usable. it should be discarded rather
    /// than returned to the pool on drop.
    fn mark_destroy(&mut self);
    /// returns true when the connection is marked as destroy-on-drop.
    fn is_marked_destroy(&self) -> bool;
}

/// implementation hook for [SharedLease]. a custom pool implementation
/// provides this trait on its leased connection type.
pub trait SharedLeaseInner: Send + Sync {
    /// immutable access to the underlying shared connection.
    fn get(&self) -> &ConnectionShared;
    /// mutable access to the underlying shared connection.
    fn get_mut(&mut self) -> &mut ConnectionShared;
    /// mark the connection as not re-usable. it should be discarded rather
    /// than returned to the pool on drop.
    fn mark_destroy(&mut self);
    /// returns true when the connection is marked as destroy-on-drop.
    fn is_marked_destroy(&self) -> bool;
}

/// an exclusive connection lease.
///
/// derefs to [ConnectionExclusive] so the caller can drive h1 protocol
/// directly on it. dropping the lease returns the connection to the
/// originating pool (or discards it if [ExclusiveLease::destroy_on_drop]
/// has been called). a lease produced by [PoolRequest::spawn] is not
/// attached to any cache — dropping it closes the connection.
pub struct ExclusiveLease {
    inner: Box<dyn ExclusiveLeaseInner>,
}

impl ExclusiveLease {
    /// construct a lease from a user provided [ExclusiveLeaseInner] implementation.
    pub fn new<I>(inner: I) -> Self
    where
        I: ExclusiveLeaseInner + 'static,
    {
        Self { inner: Box::new(inner) }
    }

    /// mark the leased connection as not re-usable.
    pub fn destroy_on_drop(&mut self) {
        self.inner.mark_destroy();
    }

    /// returns true when the lease has been marked as destroy-on-drop.
    pub fn is_destroy_on_drop(&self) -> bool {
        self.inner.is_marked_destroy()
    }
}

impl Deref for ExclusiveLease {
    type Target = ConnectionExclusive;

    fn deref(&self) -> &Self::Target {
        self.inner.get()
    }
}

impl DerefMut for ExclusiveLease {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.get_mut()
    }
}

/// a shared connection lease.
///
/// derefs to [ConnectionShared] so the caller can drive h2/h3 protocol on
/// it. shared connections are cloneable reference-counted handles, so the
/// drop behavior of a lease only affects pool bookkeeping (whether the
/// connection stays reachable through the pool or is evicted).
pub struct SharedLease {
    inner: Box<dyn SharedLeaseInner>,
}

impl SharedLease {
    /// construct a lease from a user provided [SharedLeaseInner] implementation.
    pub fn new<I>(inner: I) -> Self
    where
        I: SharedLeaseInner + 'static,
    {
        Self { inner: Box::new(inner) }
    }

    /// mark the leased connection as not re-usable.
    pub fn destroy_on_drop(&mut self) {
        self.inner.mark_destroy();
    }
}

impl Deref for SharedLease {
    type Target = ConnectionShared;

    fn deref(&self) -> &Self::Target {
        self.inner.get()
    }
}

impl DerefMut for SharedLease {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.get_mut()
    }
}

// -----------------------------------------------------------------------------
// plain lease inners — used by PoolRequest::spawn, not attached to any cache
// -----------------------------------------------------------------------------

struct PlainExclusive {
    conn: Option<ConnectionExclusive>,
    destroy: bool,
}

impl PlainExclusive {
    fn new(conn: ConnectionExclusive) -> Self {
        Self {
            conn: Some(conn),
            destroy: false,
        }
    }
}

impl ExclusiveLeaseInner for PlainExclusive {
    fn get(&self) -> &ConnectionExclusive {
        self.conn.as_ref().expect("PlainExclusive must contain a connection")
    }

    fn get_mut(&mut self) -> &mut ConnectionExclusive {
        self.conn.as_mut().expect("PlainExclusive must contain a connection")
    }

    fn mark_destroy(&mut self) {
        self.destroy = true;
    }

    fn is_marked_destroy(&self) -> bool {
        self.destroy
    }
}

struct PlainShared {
    conn: ConnectionShared,
    destroy: bool,
}

impl PlainShared {
    fn new(conn: ConnectionShared) -> Self {
        Self { conn, destroy: false }
    }
}

impl SharedLeaseInner for PlainShared {
    fn get(&self) -> &ConnectionShared {
        &self.conn
    }

    fn get_mut(&mut self) -> &mut ConnectionShared {
        &mut self.conn
    }

    fn mark_destroy(&mut self) {
        self.destroy = true;
    }

    fn is_marked_destroy(&self) -> bool {
        self.destroy
    }
}

// -----------------------------------------------------------------------------
// connection establishment primitive — shared by DefaultPool and PoolRequest::spawn
// -----------------------------------------------------------------------------

/// outcome of a single establishment attempt.
enum EstablishOutcome {
    Exclusive {
        conn: ConnectionExclusive,
        version: Version,
    },
    Shared {
        conn: ConnectionShared,
        version: Version,
    },
    /// the requested version could not be established and the caller should
    /// retry at the indicated lower version (h3→h2 or h2c→h1 fallback).
    RetryLower(Version),
}

/// run dns resolve + transport connect + tls handshake + (when requested)
/// h2 / h3 protocol handshake for a single version attempt.
#[allow(unused_variables, unused_mut, unreachable_code)]
async fn establish(
    client: &Client,
    connect: &mut Connect<'_>,
    version: Version,
    allow_h2c_downgrade: bool,
) -> Result<EstablishOutcome, Error> {
    #[cfg(feature = "http2")]
    use crate::uri::Uri as UriKind;
    #[cfg(feature = "http3")]
    use crate::{error::TimeoutError, timeout::Timeout};

    match version {
        Version::HTTP_3 => {
            #[cfg(feature = "http3")]
            {
                let mut timer = Box::pin(tokio::time::sleep(client.timeout_config.resolve_timeout));

                Service::call(&client.resolver, connect)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| TimeoutError::Resolve)??;

                timer
                    .as_mut()
                    .reset(tokio::time::Instant::now() + client.timeout_config.connect_timeout);

                match crate::h3::proto::connect(&client.h3_client, connect.addrs(), connect.hostname())
                    .timeout(timer.as_mut())
                    .await
                {
                    Ok(Ok(conn)) => Ok(EstablishOutcome::Shared {
                        conn: conn.into(),
                        version: Version::HTTP_3,
                    }),
                    _ => {
                        #[cfg(feature = "http2")]
                        {
                            Ok(EstablishOutcome::RetryLower(Version::HTTP_2))
                        }
                        #[cfg(not(feature = "http2"))]
                        {
                            Ok(EstablishOutcome::RetryLower(Version::HTTP_11))
                        }
                    }
                }
            }

            #[cfg(not(feature = "http3"))]
            {
                Err(crate::error::FeatureError::Http3NotEnabled.into())
            }
        }
        Version::HTTP_2 => {
            #[cfg(feature = "http2")]
            {
                let mut timer = Box::pin(tokio::time::sleep(client.timeout_config.resolve_timeout));
                let (conn, alpn_version) = client.make_exclusive(connect, &mut timer, Version::HTTP_2).await?;

                if alpn_version == Version::HTTP_2 {
                    let is_h2c = matches!(connect.uri, UriKind::Tcp(_));

                    match crate::h2::proto::handshake(conn).await {
                        Ok(conn) => Ok(EstablishOutcome::Shared {
                            conn: conn.into(),
                            version: Version::HTTP_2,
                        }),
                        Err(e) if is_h2c && allow_h2c_downgrade => {
                            #[cfg(not(feature = "http1"))]
                            {
                                return Err(e.into());
                            }
                            #[cfg(feature = "http1")]
                            {
                                let _ = e;
                                Ok(EstablishOutcome::RetryLower(Version::HTTP_11))
                            }
                        }
                        Err(e) => Err(e.into()),
                    }
                } else {
                    #[cfg(not(feature = "http1"))]
                    {
                        Err(crate::error::FeatureError::Http1NotEnabled.into())
                    }
                    #[cfg(feature = "http1")]
                    {
                        Ok(EstablishOutcome::Exclusive {
                            conn,
                            version: alpn_version,
                        })
                    }
                }
            }

            #[cfg(not(feature = "http2"))]
            {
                Err(crate::error::FeatureError::Http2NotEnabled.into())
            }
        }
        ver => {
            let mut timer = Box::pin(tokio::time::sleep(client.timeout_config.resolve_timeout));
            let (conn, _) = client.make_exclusive(connect, &mut timer, ver).await?;
            Ok(EstablishOutcome::Exclusive { conn, version: ver })
        }
    }
}

// -----------------------------------------------------------------------------
// default pool implementation
// -----------------------------------------------------------------------------

impl ExclusiveLeaseInner for exclusive::Conn<ConnectionKey, ConnectionExclusive> {
    fn get(&self) -> &ConnectionExclusive {
        self
    }

    fn get_mut(&mut self) -> &mut ConnectionExclusive {
        self
    }

    fn mark_destroy(&mut self) {
        self.set_destroy_on_drop();
    }

    fn is_marked_destroy(&self) -> bool {
        self.is_destroy_on_drop()
    }
}

impl SharedLeaseInner for shared::Conn<ConnectionKey, ConnectionShared> {
    fn get(&self) -> &ConnectionShared {
        &self.conn
    }

    fn get_mut(&mut self) -> &mut ConnectionShared {
        &mut self.conn
    }

    fn mark_destroy(&mut self) {
        self.set_destroy_on_drop();
    }

    fn is_marked_destroy(&self) -> bool {
        self.is_destroy_on_drop()
    }
}

pub(crate) struct DefaultPool {
    exclusive: exclusive::Pool<ConnectionKey, ConnectionExclusive>,
    shared: shared::Pool<ConnectionKey, ConnectionShared>,
}

impl DefaultPool {
    pub(crate) fn new(cap: usize, keep_alive_idle: Duration, keep_alive_born: Duration) -> Self {
        Self {
            exclusive: exclusive::Pool::new(cap, keep_alive_idle, keep_alive_born),
            shared: shared::Pool::with_capacity(cap),
        }
    }
}

/// construct the default [PoolService] implementation.
pub(crate) fn base_pool(cap: usize, keep_alive_idle: Duration, keep_alive_born: Duration) -> PoolService {
    Box::new(DefaultPool::new(cap, keep_alive_idle, keep_alive_born))
}

impl<'a, 'c> Service<PoolRequest<'a, 'c>> for DefaultPool {
    type Response = Lease;
    type Error = Error;

    async fn call(&self, mut req: PoolRequest<'a, 'c>) -> Result<Self::Response, Self::Error> {
        loop {
            match req.version {
                Version::HTTP_2 | Version::HTTP_3 => match self.shared.acquire(&req.connect.uri).await {
                    shared::AcquireOutput::Conn(c) => {
                        // shared::Pool::acquire has already probed the cached
                        // entry via the Ready trait — by this point the
                        // connection is known to accept new streams.
                        return Ok(Lease::Shared {
                            conn: SharedLease::new(c),
                            version: req.version,
                        });
                    }
                    shared::AcquireOutput::Spawner(spawner) => {
                        match establish(req.client, &mut req.connect, req.version, req.allow_h2c_downgrade).await? {
                            EstablishOutcome::Shared { conn, version } => {
                                spawner.spawned(conn);
                                // re-enter loop to pick up the newly inserted shared connection.
                                req.version = version;
                            }
                            EstablishOutcome::Exclusive { conn, version } => {
                                // alpn downgraded to http/1. release shared slot and insert into
                                // exclusive cache so future callers at this key benefit too.
                                drop(spawner);
                                #[cfg(feature = "http1")]
                                {
                                    self.exclusive.try_add(&req.connect.uri, conn);
                                    req.version = version;
                                }
                                #[cfg(not(feature = "http1"))]
                                {
                                    let _ = conn;
                                    let _ = version;
                                    return Err(crate::error::FeatureError::Http1NotEnabled.into());
                                }
                            }
                            EstablishOutcome::RetryLower(lower) => {
                                drop(spawner);
                                req.version = lower;
                            }
                        }
                    }
                },
                ver => match self.exclusive.acquire(&req.connect.uri).await {
                    exclusive::AcquireOutput::Conn(c) => {
                        return Ok(Lease::Exclusive {
                            conn: ExclusiveLease::new(c),
                            version: ver,
                        });
                    }
                    exclusive::AcquireOutput::Spawner(spawner) => {
                        match establish(req.client, &mut req.connect, ver, req.allow_h2c_downgrade).await? {
                            EstablishOutcome::Exclusive { conn, .. } => {
                                spawner.spawned(conn);
                            }
                            EstablishOutcome::Shared { .. } | EstablishOutcome::RetryLower(_) => {
                                // http/1 establishment never produces shared/retry outcomes.
                                unreachable!("establish at http/1 returned unexpected outcome");
                            }
                        }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
#[cfg(feature = "http1")]
mod h1_tests {
    use core::time::Duration;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::{io::AsyncWriteExt, net::TcpListener};

    use crate::Client;

    /// raw h1 server that replies with a well-formed Content-Length: 5 body
    /// ("hello"), then after a short delay writes trailing garbage bytes into
    /// the socket. the delay ensures the extra bytes arrive after the client
    /// has consumed the body and returned the connection to the pool.
    async fn serve_with_trailing_bytes(tcp: &mut tokio::net::TcpStream, extra_sent: Arc<tokio::sync::Notify>) {
        use tokio::io::AsyncReadExt;

        let mut buf = vec![0u8; 4096];
        loop {
            let n = tcp.read(&mut buf).await.unwrap();
            if n == 0 {
                return;
            }
            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        tcp.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
            .await
            .unwrap();
        tcp.flush().await.unwrap();

        // wait for the client to consume the body and return the connection
        // to the pool, then inject trailing garbage.
        tokio::time::sleep(Duration::from_millis(100)).await;
        tcp.write_all(b"EXTRA").await.unwrap();
        tcp.flush().await.unwrap();
        extra_sent.notify_one();
    }

    /// raw h1 server that replies with a well-formed response (no trailing garbage).
    async fn serve_clean(tcp: &mut tokio::net::TcpStream) {
        use tokio::io::AsyncReadExt;

        let mut buf = vec![0u8; 4096];
        loop {
            let n = tcp.read(&mut buf).await.unwrap();
            if n == 0 {
                return;
            }
            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        tcp.write_all(response).await.unwrap();
        tcp.flush().await.unwrap();
    }

    #[tokio::test]
    async fn h1_leftover_bytes_evict_pool_entry() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/");

        let accept_count = Arc::new(AtomicUsize::new(0));
        let extra_sent = Arc::new(tokio::sync::Notify::new());

        let server = {
            let accept_count = accept_count.clone();
            let extra_sent = extra_sent.clone();
            tokio::spawn(async move {
                // first connection: sends response then injects trailing garbage
                let (mut tcp1, _) = listener.accept().await.unwrap();
                accept_count.fetch_add(1, Ordering::SeqCst);
                serve_with_trailing_bytes(&mut tcp1, extra_sent).await;

                // second connection: the pool should have evicted the first
                // connection due to leftover readable bytes, so the client
                // opens a fresh TCP connection.
                let (mut tcp2, _) = listener.accept().await.unwrap();
                accept_count.fetch_add(1, Ordering::SeqCst);
                serve_clean(&mut tcp2).await;

                // keep sockets alive so the client can finish reading
                tokio::time::sleep(Duration::from_millis(500)).await;
            })
        };

        let client = Client::builder().finish();

        // first request: fully consume the 5-byte body
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), 200);
        let body = res.body().await.unwrap();
        assert_eq!(&body, b"hello");
        // connection is returned to pool; server will inject trailing bytes shortly

        // wait until the server has written the extra bytes into the socket
        extra_sent.notified().await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        // second request: the Ready check should detect leftover readable
        // data and discard the tainted connection, opening a fresh one.
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), 200);
        let body = res.body().await.unwrap();
        assert_eq!(&body, b"ok");

        drop(client);

        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("server task did not finish")
            .unwrap();

        assert_eq!(
            accept_count.load(Ordering::SeqCst),
            2,
            "client should have opened a second TCP connection after detecting leftover bytes"
        );
    }
}

#[cfg(test)]
#[cfg(feature = "http2")]
mod tests {
    use core::time::Duration;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::{net::TcpListener, sync::Notify};

    use crate::{Client, http::Version};

    /// drives a single h2 server connection: accepts one request, replies with
    /// 200, optionally sends GOAWAY, then drains the connection until it closes.
    async fn serve_one(tcp: tokio::net::TcpStream, send_goaway: bool) {
        let mut conn = h2::server::handshake(tcp).await.unwrap();
        if let Some(req) = conn.accept().await {
            let (_req, mut respond) = req.unwrap();
            let resp = crate::http::Response::builder().status(200).body(()).unwrap();
            respond.send_response(resp, true).unwrap();
        }
        if send_goaway {
            conn.graceful_shutdown();
        }
        while conn.accept().await.is_some() {}
    }

    #[tokio::test]
    async fn h2_goaway_evicts_pool_entry() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/");

        let accept_count = Arc::new(AtomicUsize::new(0));
        let goaway_drained = Arc::new(Notify::new());

        let server = {
            let accept_count = accept_count.clone();
            let goaway_drained = goaway_drained.clone();
            tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                accept_count.fetch_add(1, Ordering::SeqCst);
                serve_one(tcp, true).await;
                goaway_drained.notify_one();

                let (tcp, _) = listener.accept().await.unwrap();
                accept_count.fetch_add(1, Ordering::SeqCst);
                serve_one(tcp, false).await;
            })
        };

        let client = Client::builder().finish();

        let res = client.get(&url).version(Version::HTTP_2).send().await.unwrap();
        assert_eq!(res.status(), 200);
        drop(res);

        // wait for the server side to send GOAWAY and the client connection task
        // to drain it, so the cached SendRequest reports the connection unusable.
        goaway_drained.notified().await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        let res = client.get(&url).version(Version::HTTP_2).send().await.unwrap();
        assert_eq!(res.status(), 200);
        drop(res);
        // dropping the client releases the cached SendRequest, letting the
        // server's second connection drain to completion.
        drop(client);

        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("server task did not finish")
            .unwrap();

        assert_eq!(
            accept_count.load(Ordering::SeqCst),
            2,
            "client should have established a fresh tcp connection after GOAWAY"
        );
    }
}
