use core::{net::SocketAddr, pin::Pin};

use futures_core::stream::Stream;
use tokio::time::{Instant, Sleep};
use xitca_io::net::{TcpSocket, TcpStream};

use crate::{
    body::{BodyError, BoxBody},
    builder::ClientBuilder,
    bytes::Bytes,
    connect::Connect,
    connection::{ConnectionExclusive, ConnectionKey, ConnectionShared},
    date::DateTimeService,
    error::{Error, ResolveError, TimeoutError},
    http::{self, Method, Version, uri},
    http_tunnel::HttpTunnelRequest,
    pool,
    request::RequestBuilder,
    resolver::ResolverService,
    service::HttpService,
    timeout::{Timeout, TimeoutConfig},
    tls::connector::Connector,
    upgrade::UpgradeRequest,
    uri::Uri,
};

/// http client type used for sending [Request] and receive [Response].
///
/// [Request]: crate::request::RequestBuilder
/// [Response]: crate::response::Response
pub struct Client {
    pub(crate) exclusive_pool: pool::exclusive::Pool<ConnectionKey, ConnectionExclusive>,
    pub(crate) shared_pool: pool::shared::Pool<ConnectionKey, ConnectionShared>,
    pub(crate) connector: Connector,
    pub(crate) resolver: ResolverService,
    pub(crate) timeout_config: TimeoutConfig,
    pub(crate) max_http_version: Version,
    pub(crate) local_addr: Option<SocketAddr>,
    pub(crate) date_service: DateTimeService,
    pub(crate) service: HttpService,
    #[cfg(feature = "http3")]
    pub(crate) h3_client: h3_quinn::quinn::Endpoint,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! method {
    ($method: tt, $method2: tt) => {
        #[doc = concat!("Start a new [Method::",stringify!($method2),"] request with empty request body.")]
        pub fn $method<U>(&self, uri: U) -> RequestBuilder<'_>
        where
            uri::Uri: TryFrom<U>,
            Error: From<<uri::Uri as TryFrom<U>>::Error>,
        {
            self.request_builder(uri, Method::$method2)
        }
    };
}

impl Client {
    /// Construct a new Client with default setting.
    pub fn new() -> Self {
        Self::builder().finish()
    }

    /// Start a new ClientBuilder and with customizable configuration.
    ///
    /// See [ClientBuilder] for detail.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Start a new HTTP request with given [http::Request].
    #[inline]
    pub fn request<B, E>(&self, req: http::Request<B>) -> RequestBuilder<'_>
    where
        B: Stream<Item = Result<Bytes, E>> + Send + 'static,
        BodyError: From<E>,
    {
        RequestBuilder::new(req, self)
    }

    method!(get, GET);
    method!(post, POST);
    method!(put, PUT);
    method!(patch, PATCH);
    method!(delete, DELETE);
    method!(options, OPTIONS);
    method!(head, HEAD);

    fn request_builder<U>(&self, url: U, method: Method) -> RequestBuilder<'_>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        let mut req = http::Request::new(BoxBody::default());
        *req.method_mut() = method;
        *req.version_mut() = self.max_http_version;

        let err = uri::Uri::try_from(url).map(|uri| *req.uri_mut() = uri).err();

        let mut builder = self.request(req);

        if let Some(e) = err {
            builder.push_error(e.into());
        }

        builder
    }

    /// Start a new connect request for http tunnel.
    ///
    /// # Example
    /// ```rust
    /// use xitca_client::{Client, bytes::Bytes};
    ///
    /// # async fn _main() -> Result<(), xitca_client::error::Error> {
    /// // construct a new client and initialize connect request.
    /// let client = Client::new();
    /// let mut http_tunnel = client.connect("http://localhost:8080").send().await?;
    ///
    /// // http_tunnel is tunnel connection type exposing Stream and Sink trait
    /// // interfaces for 2 way bytes data communicating.
    ///
    /// // import Stream trait and call it's method on tunnel to receive bytes.
    /// use futures::StreamExt;
    /// if let Some(Ok(_)) = http_tunnel.next().await {
    ///     // received bytes data.
    /// }
    ///
    /// // import Sink trait and call it's method on tunnel to send bytes data.
    /// use futures::SinkExt;
    /// // send bytes data.
    /// http_tunnel.send(b"996").await?;
    ///
    /// // tunnel support split sending/receiving task into different parts to enable concurrent bytes data handling.
    /// let (mut write, mut read) = http_tunnel.split();
    ///
    /// // read part can operate with Stream trait implement.
    /// if let Some(Ok(_)) = read.next().await {
    ///     // received bytes data.
    /// }
    ///
    /// // write part can operate with Sink trait implement.
    /// write.send(b"996").await?;
    ///
    /// let mut http_tunnel = client.connect("http://localhost:8080").send().await?;    
    ///
    /// // import AsyncIo trait and use http tunnel as io type directly.
    /// use xitca_io::io::{Interest, AsyncIo};
    /// let mut tunnel = http_tunnel.into_inner(); // acquire inner tunnel type that impl AsyncIo trait.
    ///
    /// // wait for tunnel ready to read
    /// tunnel.ready(Interest::READABLE).await?;
    ///
    /// let mut buf = [0; 1024];
    ///
    /// // use std::io to read from tunnel.
    /// let n = std::io::Read::read(&mut tunnel, &mut buf)?;
    /// println!("read bytes: {:?}", &buf[..n]);
    ///
    /// // wait for tunnel ready to write
    /// tunnel.ready(Interest::WRITABLE).await?;
    ///
    /// // use std::io to write to tunnel.
    /// let _n = std::io::Write::write(&mut tunnel, &buf[..n])?;
    ///
    /// // import compat type if you want tunnel to be used with tokio 1.0.
    /// let tunnel = xitca_io::io::PollIoAdapter(tunnel);
    ///
    /// // from this point on tunnel is able to used with tokio::io::{AsyncRead, AsyncWrite} traits.
    ///
    /// Ok(())
    /// # }
    /// ```
    pub fn connect<U>(&self, url: U) -> HttpTunnelRequest<'_>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        self.request_builder(url, Method::CONNECT).mutate_marker()
    }

    #[cfg(feature = "http1")]
    /// Start a new upgrade request.
    ///
    /// # Example
    /// ```rust
    /// use xitca_client::{Client, bytes::Bytes, http::Method};
    ///
    /// async fn _main() -> Result<(), xitca_client::error::Error> {
    /// // construct a new client and initialize connect request.
    /// let client = Client::new();
    /// let mut upgrade_response = client
    ///     .upgrade("http://localhost:8080", Method::GET)
    ///     .protocol(["protocol1", "protocol2"])
    ///     .send().await?
    /// ;
    ///
    /// if let Some(upgrade) = upgrade_response.headers.get(xitca_client::http::header::UPGRADE) {
    ///    // check which protocol it was upgraded to
    /// }
    ///
    /// // upgrade_response is a response that contains the http request head and tunnel connection.
    ///
    /// // import Stream trait and call it's method on tunnel to receive bytes.
    /// use futures::StreamExt;
    /// if let Some(Ok(_)) = upgrade_response.tunnel().next().await {
    ///     // received bytes data.
    /// }
    ///
    /// // import Sink trait and call it's method on tunnel to send bytes data.
    /// use futures::SinkExt;
    /// // send bytes data.
    /// upgrade_response.tunnel().send(b"996").await?;
    ///
    /// // tunnel support split sending/receiving task into different parts to enable concurrent bytes data handling.
    /// let (_head, mut tunnel) = upgrade_response.into_parts();
    /// let (mut write, mut read) = tunnel.split();
    ///
    /// // read part can operate with Stream trait implement.
    /// if let Some(Ok(_)) = read.next().await {
    ///     // received bytes data.
    /// }
    ///
    /// // write part can operate with Sink trait implement.
    /// write.send(b"996").await?;
    ///
    /// Ok(())
    /// # }
    /// ```
    pub fn upgrade<U>(&self, url: U, method: Method) -> UpgradeRequest<'_>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        self.request_builder(url, method)
            .version(Version::HTTP_11)
            .mutate_marker()
    }

    #[cfg(all(feature = "websocket", feature = "http1"))]
    /// Start a new websocket request.
    ///
    /// # Example
    /// ```rust
    /// use xitca_client::{ws::Message, Client};
    ///
    /// # async fn _main() -> Result<(), xitca_client::error::Error> {
    /// // construct a new client and initialize websocket request.
    /// let client = Client::new();
    /// let mut ws = client.ws("ws://localhost:8080").send().await?;
    ///
    /// // ws is websocket connection type exposing Stream and Sink trait
    /// // interfaces for 2 way websocket message communicating.
    ///
    /// // import Stream trait and call it's method on ws to receive message.
    /// use futures::StreamExt;
    /// if let Some(Ok(_)) = ws.next().await {
    ///     // received message.
    /// }
    ///
    /// // import Sink trait and call it's method on ws to send message.
    /// use futures::SinkExt;
    /// // send text message.
    /// ws.send(Message::Text("996".into())).await?;
    ///
    /// // ws support split sending/receiving task into different parts to enable concurrent message handling.
    /// let (mut write, mut read) = ws.split();
    ///
    /// // read part can operate with Stream trait implement.
    /// if let Some(Ok(_)) = read.next().await {
    ///     // received message.
    /// }
    ///
    /// // write part can operate with Sink trait implement.
    /// write.send(Message::Text("996".into())).await?;
    ///
    /// Ok(())
    /// # }
    /// ```
    pub fn ws<U>(&self, url: U) -> crate::ws::WsRequest<'_>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        self.get(url).version(Version::HTTP_11).mutate_marker()
    }

    #[cfg(all(feature = "websocket", feature = "http2"))]
    /// Start a new websocket request with HTTP/2.
    pub fn ws2<U>(&self, url: U) -> crate::ws::WsRequest<'_>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        self.get(url).version(Version::HTTP_2).mutate_marker()
    }
}

impl Client {
    // make exclusive connection that can be inserted into exclusive connection pool.
    // an expected http version for connection is received and a final http version determined
    // by server side alpn protocol would be returned.
    // when the returned version is HTTP_2 the exclusive connection can be upgraded to shared
    // connection for http2.
    pub(crate) async fn make_exclusive(
        &self,
        connect: &mut Connect<'_>,
        timer: &mut Pin<Box<Sleep>>,
        expected_version: Version,
    ) -> Result<(ConnectionExclusive, Version), Error> {
        match connect.uri {
            Uri::Tcp(_) | Uri::Tls(_) => {
                let conn = self.make_tcp(connect, timer).await?;

                if matches!(connect.uri, Uri::Tcp(_)) {
                    return Ok((conn, expected_version));
                }

                timer
                    .as_mut()
                    .reset(Instant::now() + self.timeout_config.tls_connect_timeout);

                let (conn, version) = self
                    .connector
                    .call((connect.sni_hostname(), conn))
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| TimeoutError::TlsHandshake)??;

                Ok((conn, version))
            }
            Uri::Unix(_) => self
                .make_unix(connect, timer)
                .await
                .map(|conn| (conn, expected_version)),
        }
    }

    async fn make_tcp(
        &self,
        connect: &mut Connect<'_>,
        timer: &mut Pin<Box<Sleep>>,
    ) -> Result<ConnectionExclusive, Error> {
        self.resolver
            .call(connect)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::Resolve)??;

        timer
            .as_mut()
            .reset(Instant::now() + self.timeout_config.connect_timeout);

        let stream = self
            .make_tcp_inner(connect)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::Connect)??;

        // TODO: make nodelay configurable?
        let _ = stream.set_nodelay(true);

        Ok(Box::new(stream))
    }

    async fn make_tcp_inner(&self, connect: &Connect<'_>) -> Result<TcpStream, Error> {
        let mut iter = connect.addrs();

        let mut addr = iter.next().ok_or_else(|| ResolveError::new(connect.hostname()))?;

        // try to connect with all addresses resolved by dns resolver.
        // return the last error when all are fail to be connected.
        loop {
            match self.maybe_connect_with_local_addr(addr).await {
                Ok(stream) => return Ok(stream),
                Err(e) => match iter.next() {
                    Some(a) => addr = a,
                    None => return Err(e),
                },
            }
        }
    }

    async fn maybe_connect_with_local_addr(&self, addr: SocketAddr) -> Result<TcpStream, Error> {
        match self.local_addr {
            Some(local_addr) => {
                let socket = match local_addr {
                    SocketAddr::V4(_) => {
                        let socket = TcpSocket::new_v4()?;
                        socket.bind(local_addr)?;
                        socket
                    }
                    SocketAddr::V6(_) => {
                        let socket = TcpSocket::new_v6()?;
                        socket.bind(local_addr)?;
                        socket
                    }
                };
                let stream = socket.connect(addr).await?;
                Ok(TcpStream::from(stream))
            }
            None => TcpStream::connect(addr).await.map_err(Into::into),
        }
    }

    async fn make_unix(
        &self,
        _connect: &Connect<'_>,
        timer: &mut Pin<Box<Sleep>>,
    ) -> Result<ConnectionExclusive, Error> {
        timer
            .as_mut()
            .reset(Instant::now() + self.timeout_config.connect_timeout);

        #[cfg(unix)]
        {
            let path = format!(
                "/{}{}",
                _connect.uri.authority().unwrap().as_str(),
                _connect.uri.path_and_query().unwrap().as_str()
            );

            let stream = xitca_io::net::UnixStream::connect(path)
                .timeout(timer.as_mut())
                .await
                .map_err(|_| TimeoutError::Connect)??;

            Ok(Box::new(stream))
        }

        #[cfg(not(unix))]
        {
            unimplemented!("only unix supports unix domain socket")
        }
    }
}

#[cfg(feature = "compress")]
#[cfg(feature = "openssl")]
#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn connect_google() {
        let res = Client::builder()
            .middleware(crate::middleware::FollowRedirect::new)
            .middleware(crate::middleware::Decompress::new)
            .openssl()
            .finish()
            .get("https://www.google.com/")
            .send()
            .await
            .unwrap()
            .body()
            .await
            .unwrap();
        println!("{}", String::from_utf8_lossy(&res));
    }
}
