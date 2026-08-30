//! load balancing connection pool as a [Service] layer.
//!
//! [BalancePool] resolves the connection target to one or more [SocketAddr]
//! before consulting a connection pool: when a [PoolRequest] does not already
//! carry a forced address (see [RequestBuilder::address]) the pool runs the
//! client's resolver eagerly, then -- when more than one address comes back --
//! consults a [LoadBalance] strategy to pick a single address for the request.
//! The chosen address is written back onto [Connect] so connection
//! establishment only ever targets it, and the resulting connection is cached
//! independently per `(connection target, address)` pair using the same
//! exclusive (http/1) / shared (http/2, http/3) pool machinery the default
//! pool relies on.
//!
//! Three [LoadBalance] strategies are provided: [RandomBalance], picking a
//! uniformly random address; [LeastUsedBalance], preferring the address with
//! the fewest currently leased connections; and [HashBalance], deterministically
//! mapping a request to an address through a user controllable [HashKey].
//!
//! [Service]: crate::service::Service
//! [RequestBuilder::address]: crate::request::RequestBuilder::address

use core::{
    hash::{BuildHasher, Hash, Hasher},
    net::SocketAddr,
    ops::{Deref, DerefMut},
    time::Duration,
};

use std::{
    collections::{HashMap, hash_map::RandomState},
    hash::DefaultHasher,
    sync::{Arc, Mutex},
};

use crate::{
    connect::Connect,
    connection::{ConnectionExclusive, ConnectionKey, ConnectionShared},
    error::{Error, ResolveError, TimeoutError},
    http::Version,
    pool::{
        exclusive,
        service::{Lease, Leaser, PoolRequest, SpawnOutCome},
        shared,
    },
    service::Service,
    timeout::Timeout,
    uri::Uri,
};

// -----------------------------------------------------------------------------
// load balancing strategy
// -----------------------------------------------------------------------------

/// strategy for selecting a single [SocketAddr] from a set of candidates that a
/// connection target resolved to.
///
/// implementations are consulted by [BalancePool] whenever a [PoolRequest]'s
/// connection target resolves to more than one address, letting requests be
/// spread across backend instances while still reusing the crate's exclusive
/// (http/1) and shared (http/2 / http/3) connection caches underneath -- each
/// selected address gets its own cached connection(s), keyed independently.
pub trait LoadBalance: Send + Sync {
    /// per-lease bookkeeping released (via [Drop]) once the leased connection is
    /// dropped. stateless strategies can use `()`.
    type Guard: Send + Sync + 'static;

    /// select an address for `req`'s connection target from `addrs`.
    ///
    /// `addrs` is guaranteed to contain at least two entries: [BalancePool] only
    /// calls this when there is an actual choice to make. besides [PoolRequest::connect],
    /// `req` also exposes the original request's [PoolRequest::uri] and
    /// [PoolRequest::extensions] so strategies can factor in e.g. the request
    /// path or a routing hint stashed in an extension.
    fn select(&self, req: &PoolRequest<'_, '_>, addrs: &[SocketAddr]) -> (SocketAddr, Self::Guard);
}

/// [LoadBalance] strategy that picks a uniformly random address for every request.
#[derive(Debug, Default, Clone, Copy)]
pub struct RandomBalance;

impl LoadBalance for RandomBalance {
    type Guard = ();

    fn select(&self, _: &PoolRequest<'_, '_>, addrs: &[SocketAddr]) -> (SocketAddr, Self::Guard) {
        (addrs[random_index(addrs.len())], ())
    }
}

// cheap pseudo random index generator that avoids pulling in a `rand` dependency.
// `RandomState` derives a fresh pair of siphash keys from OS randomness (mixed
// with a per thread counter) every time it's constructed, so hashing with a
// freshly built hasher yields a different value on each call.
fn random_index(len: usize) -> usize {
    debug_assert_ne!(len, 0);
    let hash = RandomState::new().build_hasher().finish();
    (hash as usize) % len
}

/// [LoadBalance] strategy that tracks how many leased connections are currently
/// outstanding per address and always picks the least loaded one.
///
/// ties are broken by iteration order of the candidate address slice.
#[derive(Clone, Default)]
pub struct LeastUsedBalance {
    counts: Arc<Mutex<HashMap<SocketAddr, usize>>>,
}

impl LeastUsedBalance {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LoadBalance for LeastUsedBalance {
    type Guard = LeastUsedGuard;

    fn select(&self, _: &PoolRequest<'_, '_>, addrs: &[SocketAddr]) -> (SocketAddr, Self::Guard) {
        let mut counts = self.counts.lock().unwrap();

        let addr = addrs
            .iter()
            .copied()
            .min_by_key(|addr| counts.get(addr).copied().unwrap_or(0))
            .expect("addrs must not be empty");

        *counts.entry(addr).or_insert(0) += 1;

        drop(counts);

        (
            addr,
            LeastUsedGuard {
                counts: self.counts.clone(),
                addr,
            },
        )
    }
}

/// drop guard for [LeastUsedBalance] that releases the in-use slot it accounted
/// for on selection.
pub struct LeastUsedGuard {
    counts: Arc<Mutex<HashMap<SocketAddr, usize>>>,
    addr: SocketAddr,
}

impl Drop for LeastUsedGuard {
    fn drop(&mut self) {
        if let Some(count) = self.counts.lock().unwrap().get_mut(&self.addr) {
            *count = count.saturating_sub(1);
        }
    }
}

/// trait controlling which parts of a request contribute to the hash
/// [HashBalance] uses to deterministically pick an address.
///
/// implement this to choose what should determine the address mapping -- e.g.
/// hash on the hostname for sticky per-domain routing, or fold in
/// [PoolRequest::uri]'s path or a value from [PoolRequest::extensions] for
/// finer grained / request-aware distribution.
pub trait HashKey: Send + Sync {
    /// feed the parts of `req` that should influence address selection into `state`.
    fn hash_key<H: Hasher>(&self, req: &PoolRequest<'_, '_>, state: &mut H);
}

/// default [HashKey] that hashes on the connection target's hostname, so every
/// request for the same host consistently maps to the same address.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostHashKey;

impl HashKey for HostHashKey {
    fn hash_key<H: Hasher>(&self, req: &PoolRequest<'_, '_>, state: &mut H) {
        req.connect.hostname().hash(state);
    }
}

/// [LoadBalance] strategy that deterministically maps a request to an address by
/// hashing parts of the connection target chosen through a [HashKey].
pub struct HashBalance<K = HostHashKey> {
    key: K,
}

impl HashBalance<HostHashKey> {
    /// construct a [HashBalance] hashing on the connection target's hostname.
    pub fn new() -> Self {
        Self { key: HostHashKey }
    }
}

impl Default for HashBalance<HostHashKey> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> HashBalance<K>
where
    K: HashKey,
{
    /// construct a [HashBalance] using a custom [HashKey] implementation.
    pub fn with_key(key: K) -> Self {
        Self { key }
    }
}

impl<K> LoadBalance for HashBalance<K>
where
    K: HashKey,
{
    type Guard = ();

    fn select(&self, req: &PoolRequest<'_, '_>, addrs: &[SocketAddr]) -> (SocketAddr, Self::Guard) {
        let mut hasher = DefaultHasher::new();
        self.key.hash_key(req, &mut hasher);
        let idx = (hasher.finish() as usize) % addrs.len();
        (addrs[idx], ())
    }
}

// -----------------------------------------------------------------------------
// pool implementation
// -----------------------------------------------------------------------------

// connection cache key for [BalancePool]. extends the regular [ConnectionKey]
// with the selected address so connections to different backend instances of
// the same logical target are cached (and load balanced) independently.
// `addr` is `None` for unix domain socket targets, which carry no resolvable
// address and therefore no load balancing choice to make.
#[derive(PartialEq, Eq, Clone, Hash)]
struct LbKey {
    key: ConnectionKey,
    addr: Option<SocketAddr>,
}

impl From<(&Connect<'_>, Option<SocketAddr>)> for LbKey {
    fn from((connect, addr): (&Connect<'_>, Option<SocketAddr>)) -> Self {
        Self {
            key: ConnectionKey::from(&connect.uri),
            addr,
        }
    }
}

/// a [Service] implementation of [PoolService] that resolves a connection
/// target to one or more addresses, applies a [LoadBalance] strategy to pick
/// one when there's a choice and delegates the actual connection caching to the
/// crate's regular exclusive (http/1) and shared (http/2 / http/3) connection
/// pools, keyed per selected address.
///
/// construct with [BalancePool::new] and register through [ClientBuilder::pool].
///
/// [PoolService]: crate::pool::service::PoolService
/// [ClientBuilder::pool]: crate::builder::ClientBuilder::pool
pub struct BalancePool<B> {
    exclusive: exclusive::Pool<LbKey, ConnectionExclusive>,
    shared: shared::Pool<LbKey, ConnectionShared>,
    balance: B,
}

impl<B> BalancePool<B>
where
    B: LoadBalance,
{
    /// construct a new load balancing pool with the given [LoadBalance] strategy.
    ///
    /// `cap`, `keep_alive_idle`, `keep_alive_born` and `keep_alive_max_requests`
    /// configure the per-address exclusive (http/1) connection cache the same
    /// way [ClientBuilder::set_pool_capacity] and friends configure the default
    /// pool -- see their documentation for details. http/2 and http/3 always
    /// open a single multiplexed connection per selected address.
    ///
    /// [ClientBuilder::set_pool_capacity]: crate::builder::ClientBuilder::set_pool_capacity
    pub fn new(balance: B, cap: usize, keep_alive_idle: Duration, keep_alive_born: Duration) -> Self {
        Self {
            exclusive: exclusive::Pool::new(cap, keep_alive_idle, keep_alive_born),
            shared: shared::Pool::with_capacity(cap),
            balance,
        }
    }

    // resolve the connection target (when not already resolved/forced) and pick
    // a single address to use for it. returns `None` for unix domain socket
    // targets, which carry no resolvable address.
    async fn select_addr(
        &self,
        req: &mut PoolRequest<'_, '_>,
    ) -> Result<(Option<SocketAddr>, Option<B::Guard>), Error> {
        if matches!(req.connect.uri, Uri::Unix(_)) {
            return Ok((None, None));
        }

        if !req.connect.is_resolved() {
            let mut timer = Box::pin(tokio::time::sleep(req.client.timeout_config.resolve_timeout));
            Service::call(&req.client.resolver, &mut req.connect)
                .timeout(timer.as_mut())
                .await
                .map_err(|_| TimeoutError::Resolve)??;
        }

        let addrs = req.connect.addrs().collect::<Vec<_>>();

        let (addr, guard) = match *addrs {
            [] => return Err(ResolveError::new(req.connect.hostname()).into()),
            [addr] => (addr, None),
            ref addrs => {
                let (addr, guard) = self.balance.select(&*req, addrs);
                (addr, Some(guard))
            }
        };

        // pin connection establishment to the selected address so [PoolRequest::spawn]
        // (and any retry it triggers) only ever targets it.
        req.connect.set_addrs([addr]);

        Ok((Some(addr), guard))
    }
}

impl<'a, 'c, B> Service<PoolRequest<'a, 'c>> for BalancePool<B>
where
    B: LoadBalance,
{
    type Response = Lease;
    type Error = Error;

    async fn call(&self, mut req: PoolRequest<'a, 'c>) -> Result<Self::Response, Self::Error> {
        let (addr, guard) = self.select_addr(&mut req).await?;
        let key = LbKey::from((&req.connect, addr));

        loop {
            match req.version {
                Version::HTTP_2 | Version::HTTP_3 => match self.shared.acquire(key.clone()).await {
                    shared::AcquireOutput::Conn(conn) => {
                        return Ok(Lease::shared(Balanced { conn, _guard: guard }, req.version));
                    }
                    shared::AcquireOutput::Spawner(spawner) => match req.spawn().await? {
                        SpawnOutCome::Shared { conn, version } => {
                            spawner.spawned(conn);
                            req.version = version;
                        }
                        SpawnOutCome::Exclusive { conn, version } => {
                            // alpn downgraded to http/1. release the shared slot and cache
                            // into the exclusive pool so future callers for this address benefit too.
                            drop(spawner);
                            #[cfg(feature = "http1")]
                            {
                                self.exclusive.try_add(key.clone(), conn);
                                req.version = version;
                            }
                            #[cfg(not(feature = "http1"))]
                            {
                                let _ = conn;
                                let _ = version;
                                return Err(crate::error::FeatureError::Http1NotEnabled.into());
                            }
                        }
                        SpawnOutCome::RetryLower(lower) => {
                            drop(spawner);
                            req.version = lower;
                        }
                    },
                },
                ver => match self.exclusive.acquire(key.clone()).await {
                    exclusive::AcquireOutput::Conn(conn) => {
                        return Ok(Lease::exclusive(Balanced { conn, _guard: guard }, ver));
                    }
                    exclusive::AcquireOutput::Spawner(spawner) => match req.spawn().await? {
                        SpawnOutCome::Exclusive { conn, .. } => spawner.spawned(conn),
                        SpawnOutCome::Shared { .. } | SpawnOutCome::RetryLower(_) => {
                            // http/1 establishment never produces shared/retry outcomes.
                            unreachable!("establish at http/1 returned unexpected outcome");
                        }
                    },
                },
            }
        }
    }
}

// [Leaser] wrapper that bundles a [LoadBalance] strategy's per-lease guard with
// the underlying lease so it's released exactly when the connection is dropped.
struct Balanced<C, G> {
    conn: C,
    _guard: G,
}

impl<C, G> Deref for Balanced<C, G>
where
    C: Deref,
{
    type Target = C::Target;

    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}

impl<C, G> DerefMut for Balanced<C, G>
where
    C: DerefMut,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.conn
    }
}

impl<C, G, T> Leaser<T> for Balanced<C, G>
where
    C: Leaser<T>,
    G: Send + Sync + 'static,
{
    fn mark_destroy(&mut self) {
        self.conn.mark_destroy();
    }

    fn is_marked_destroy(&self) -> bool {
        self.conn.is_marked_destroy()
    }
}

#[cfg(test)]
#[cfg(feature = "http1")]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use std::sync::Arc;

    use tokio::{io::AsyncWriteExt, net::TcpListener, sync::Notify};

    use crate::{Client, Connect, Service, error::Error};

    use super::*;

    // raw h1 server that replies 200 OK with an empty body to every request it
    // accepts, counting how many connections it has accepted until `stop` fires.
    async fn serve_counting(listener: TcpListener, count: Arc<AtomicUsize>, stop: Arc<Notify>) {
        loop {
            tokio::select! {
                res = listener.accept() => {
                    let (mut tcp, _) = res.unwrap();
                    count.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(async move {
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
                        let _ = tcp.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
                        let _ = tcp.flush().await;
                    });
                }
                _ = stop.notified() => return,
            }
        }
    }

    // stub resolver that always reports the same fixed set of addresses,
    // standing in for a dns resolver that returned multiple records.
    struct FixedResolver(Vec<SocketAddr>);

    impl<'r, 'c> Service<&'r mut Connect<'c>> for FixedResolver {
        type Response = ();
        type Error = Error;

        async fn call(&self, connect: &'r mut Connect<'c>) -> Result<Self::Response, Self::Error> {
            connect.set_addrs(self.0.clone());
            Ok(())
        }
    }

    struct TestServers {
        addr1: SocketAddr,
        addr2: SocketAddr,
        count1: Arc<AtomicUsize>,
        count2: Arc<AtomicUsize>,
        stop: Arc<Notify>,
        handles: (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>),
    }

    impl TestServers {
        async fn spawn() -> Self {
            let l1 = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let l2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr1 = l1.local_addr().unwrap();
            let addr2 = l2.local_addr().unwrap();

            let count1 = Arc::new(AtomicUsize::new(0));
            let count2 = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(Notify::new());

            let h1 = tokio::spawn(serve_counting(l1, count1.clone(), stop.clone()));
            let h2 = tokio::spawn(serve_counting(l2, count2.clone(), stop.clone()));

            Self {
                addr1,
                addr2,
                count1,
                count2,
                stop,
                handles: (h1, h2),
            }
        }

        async fn shutdown(self) {
            self.stop.notify_waiters();
            let _ = tokio::join!(self.handles.0, self.handles.1);
        }
    }

    #[tokio::test]
    async fn random_balance_spreads_across_addresses() {
        let servers = TestServers::spawn().await;

        let client = Client::builder()
            .resolver(FixedResolver(vec![servers.addr1, servers.addr2]))
            .pool(BalancePool::new(
                RandomBalance,
                4,
                Duration::from_secs(60),
                Duration::from_secs(3600),
            ))
            .finish();

        for _ in 0..40 {
            let res = client.get("http://example.local/").send().await.unwrap();
            assert_eq!(res.status(), 200);
            let _ = res.body().await.unwrap();
        }

        // with 40 independent random picks across two addresses the chance of
        // every single one landing on the same address is astronomically small
        // (2 * 0.5^40), so observing connections on both servers confirms the
        // strategy is consulted per request rather than pinned to one address.
        assert!(servers.count1.load(Ordering::SeqCst) > 0);
        assert!(servers.count2.load(Ordering::SeqCst) > 0);

        servers.shutdown().await;
        drop(client);
    }

    #[tokio::test]
    async fn hash_balance_is_sticky_per_target() {
        let servers = TestServers::spawn().await;

        let client = Client::builder()
            .resolver(FixedResolver(vec![servers.addr1, servers.addr2]))
            .pool(BalancePool::new(
                HashBalance::new(),
                4,
                Duration::from_secs(60),
                Duration::from_secs(3600),
            ))
            .finish();

        for _ in 0..10 {
            let res = client.get("http://example.local/").send().await.unwrap();
            assert_eq!(res.status(), 200);
            let _ = res.body().await.unwrap();
        }

        // [HostHashKey] hashes only on hostname, which is constant across these
        // requests, so every one of them must deterministically map to the same
        // address -- exactly one of the two servers should ever be contacted.
        let contacted = [&servers.count1, &servers.count2]
            .iter()
            .filter(|c| c.load(Ordering::SeqCst) > 0)
            .count();
        assert_eq!(
            contacted, 1,
            "hash strategy must stick every request to a single address"
        );

        servers.shutdown().await;
        drop(client);
    }

    #[tokio::test]
    async fn hash_balance_routes_on_request_path() {
        // custom [HashKey] hashing on the request's path -- via [PoolRequest::uri]
        // -- rather than on the (constant, per [HostHashKey]) connection target,
        // demonstrating that strategies can route based on per-request data.
        struct PathHashKey;

        impl HashKey for PathHashKey {
            fn hash_key<H: Hasher>(&self, req: &PoolRequest<'_, '_>, state: &mut H) {
                req.uri.path().hash(state);
            }
        }

        let servers = TestServers::spawn().await;

        let client = Client::builder()
            .resolver(FixedResolver(vec![servers.addr1, servers.addr2]))
            .pool(BalancePool::new(
                HashBalance::with_key(PathHashKey),
                4,
                Duration::from_secs(60),
                Duration::from_secs(3600),
            ))
            .finish();

        // 20 distinct paths, hashed independently of the (constant) host: with
        // two candidate addresses the chance of every single one landing on the
        // same address is astronomically small (2 * 0.5^20), so observing
        // connections on both servers confirms the path -- not just the
        // connection target -- drives address selection.
        for i in 0..20 {
            let res = client
                .get(format!("http://example.local/path-{i}").as_str())
                .send()
                .await
                .unwrap();
            assert_eq!(res.status(), 200);
            let _ = res.body().await.unwrap();
        }

        assert!(servers.count1.load(Ordering::SeqCst) > 0);
        assert!(servers.count2.load(Ordering::SeqCst) > 0);

        servers.shutdown().await;
        drop(client);
    }

    #[tokio::test]
    async fn least_used_balance_releases_guard_on_lease_drop() {
        let servers = TestServers::spawn().await;

        let client = Client::builder()
            .resolver(FixedResolver(vec![servers.addr1, servers.addr2]))
            .pool(BalancePool::new(
                LeastUsedBalance::new(),
                4,
                Duration::from_secs(60),
                Duration::from_secs(3600),
            ))
            .finish();

        // requests run sequentially and each lease (and its [LeastUsedGuard]) is
        // dropped -- by fully consuming the body -- before the next one starts.
        // if the guard correctly decrements the in-use counter on drop, every
        // selection sees a 0/0 tie and consistently resolves to the first
        // candidate address; a stuck counter would eventually shift load to the
        // second address instead.
        for _ in 0..6 {
            let res = client.get("http://example.local/").send().await.unwrap();
            assert_eq!(res.status(), 200);
            let _ = res.body().await.unwrap();
        }

        assert!(servers.count1.load(Ordering::SeqCst) > 0);
        assert_eq!(
            servers.count2.load(Ordering::SeqCst),
            0,
            "least-used guard must release its slot when the lease drops"
        );

        servers.shutdown().await;
        drop(client);
    }
}
