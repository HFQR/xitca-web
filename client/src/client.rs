use std::{net::SocketAddr, pin::Pin};

use futures_core::stream::Stream;
use tokio::{
    net::{TcpSocket, TcpStream},
    time::{Instant, Sleep},
};

use crate::{
    body::{BodyError, BoxBody},
    builder::ClientBuilder,
    bytes::Bytes,
    connect::Connect,
    connection::{Connection, ConnectionKey},
    date::DateTimeService,
    error::{Error, TimeoutError},
    http::{self, uri, Method, Version},
    pool::Pool,
    request::RequestBuilder,
    resolver::ResolverService,
    service::HttpService,
    timeout::{Timeout, TimeoutConfig},
    tls::connector::Connector,
    uri::Uri,
};

/// http client type used for sending [Request] and receive [Response].
///
/// [Request]: crate::request::RequestBuilder
/// [Response]: crate::response::Response
pub struct Client {
    pub(crate) pool: Pool<ConnectionKey, Connection>,
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
        pub fn $method<U>(&self, url: U) -> Result<RequestBuilder<'_>, Error>
        where
            uri::Uri: TryFrom<U>,
            Error: From<<uri::Uri as TryFrom<U>>::Error>,
        {
            Ok(self.get(url)?.method(Method::$method2))
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

    /// Start a new [Method::GET] request with empty request body.
    pub fn get<U>(&self, url: U) -> Result<RequestBuilder<'_>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        let uri = uri::Uri::try_from(url)?;

        let mut req = http::Request::new(BoxBody::default());
        *req.uri_mut() = uri;
        *req.version_mut() = self.max_http_version;

        Ok(self.request(req))
    }

    method!(post, POST);
    method!(put, PUT);
    method!(patch, PATCH);
    method!(delete, DELETE);
    method!(options, OPTIONS);
    method!(head, HEAD);

    pub fn connect<U>(&self, url: U) -> Result<crate::tunnel::TunnelRequest<'_>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        self.get(url)
            .map(|req| crate::tunnel::TunnelRequest::new(req.method(Method::CONNECT)))
    }

    #[cfg(feature = "websocket")]
    /// Start a new websocket request.
    ///
    /// # Example
    /// ```rust
    /// use xitca_client::{ws::Message, Client};
    ///
    /// # async fn _main() -> Result<(), xitca_client::error::Error> {
    /// // construct a new client and initialize websocket request.
    /// let client = Client::new();
    /// let mut ws = client.ws("ws://localhost:8080")?.send().await?;
    ///
    /// // ws is websocket connection type exposing Stream and Sink trait
    /// // interfaces for 2 way websocket message processing.
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
    pub fn ws(&self, url: &str) -> Result<crate::ws::WsRequest<'_>, Error> {
        self._ws(url, Version::HTTP_11)
    }

    #[cfg(all(feature = "websocket", feature = "http2"))]
    /// Start a new websocket request with HTTP/2.
    pub fn ws2(&self, url: &str) -> Result<crate::ws::WsRequest<'_>, Error> {
        self._ws(url, Version::HTTP_2)
    }

    #[cfg(feature = "websocket")]
    fn _ws(&self, url: &str, version: Version) -> Result<crate::ws::WsRequest<'_>, Error> {
        let req = http_ws::client_request_from_uri(url, version)?.map(|_| BoxBody::default());
        let req = RequestBuilder::new(req, self);
        Ok(crate::ws::WsRequest::new(req))
    }
}

impl Client {
    pub(crate) async fn make_connection(
        &self,
        connect: &mut Connect<'_>,
        timer: &mut Pin<Box<Sleep>>,
        max_version: Version,
    ) -> Result<Connection, Error> {
        match connect.uri {
            Uri::Tcp(_) => {
                self.resolver
                    .call(connect)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| TimeoutError::Resolve)??;

                self.make_tcp(connect, timer).await.map(Into::into)
            }
            Uri::Tls(_) => {
                self.resolver
                    .call(connect)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| TimeoutError::Resolve)??;

                #[cfg(feature = "http3")]
                // TODO: find better way to discover http3.
                if max_version == Version::HTTP_3 {
                    if let Ok(conn) = self.make_h3(connect, timer).await {
                        return Ok(conn);
                    }
                }
                // Fallback to tcp if http3 failed.

                self.make_tls(connect, timer, max_version).await
            }
            #[cfg(unix)]
            Uri::Unix(uri) => self.make_unix(uri, timer).await,
        }
    }

    async fn make_tcp(&self, connect: &Connect<'_>, timer: &mut Pin<Box<Sleep>>) -> Result<TcpStream, Error> {
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

        Ok(stream)
    }

    async fn make_tcp_inner(&self, connect: &Connect<'_>) -> Result<TcpStream, Error> {
        let mut iter = connect.addrs();

        let mut addr = iter.next().ok_or(Error::Resolve)?;

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

                socket.connect(addr).await.map_err(Into::into)
            }
            None => TcpStream::connect(addr).await.map_err(Into::into),
        }
    }

    async fn make_tls(
        &self,
        connect: &Connect<'_>,
        timer: &mut Pin<Box<Sleep>>,
        max_version: Version,
    ) -> Result<Connection, Error> {
        let stream = self.make_tcp(connect, timer).await?;

        timer
            .as_mut()
            .reset(Instant::now() + self.timeout_config.tls_connect_timeout);

        let (stream, version) = self
            .connector
            .call((connect.hostname(), Box::new(stream)))
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::TlsHandshake)??;

        let version = match (version, max_version) {
            (Version::HTTP_3, Version::HTTP_2 | Version::HTTP_11 | Version::HTTP_10 | Version::HTTP_09) => max_version,
            (Version::HTTP_2, Version::HTTP_11 | Version::HTTP_10 | Version::HTTP_09) => max_version,
            (Version::HTTP_11, Version::HTTP_10 | Version::HTTP_09) => max_version,
            (Version::HTTP_10, Version::HTTP_09) => max_version,
            _ => version,
        };

        match version {
            Version::HTTP_11 => Ok(stream.into()),
            Version::HTTP_2 => {
                #[cfg(feature = "http2")]
                {
                    let connection = crate::h2::proto::handshake(stream).await?;

                    Ok(connection.into())
                }

                #[cfg(not(feature = "http2"))]
                {
                    Ok(stream.into())
                }
            }
            Version::HTTP_3 => unreachable!("HTTP_3 can not use TCP connection"),
            _ => unreachable!("HTTP_09 and HTTP_10 is impossible for tls connections"),
        }
    }

    #[cfg(feature = "http3")]
    async fn make_h3(&self, connect: &Connect<'_>, timer: &mut Pin<Box<Sleep>>) -> Result<Connection, Error> {
        timer
            .as_mut()
            .reset(Instant::now() + self.timeout_config.connect_timeout);

        let stream = self
            .make_h3_inner(connect)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::Connect)??;

        Ok(stream)
    }

    #[cfg(feature = "http3")]
    async fn make_h3_inner(&self, connect: &Connect<'_>) -> Result<Connection, Error> {
        let mut iter = connect.addrs();

        let mut addr = iter.next().ok_or(Error::Resolve)?;

        // try to connect with all addresses resolved by dns resolver.
        // return the last error when all are fail to be connected.
        loop {
            match crate::h3::proto::connect(&self.h3_client, &addr, connect.hostname()).await {
                Ok(connection) => return Ok(connection.into()),
                Err(e) => match iter.next() {
                    Some(a) => addr = a,
                    None => return Err(e.into()),
                },
            }
        }
    }

    #[cfg(unix)]
    async fn make_unix(&self, uri: &uri::Uri, timer: &mut Pin<Box<Sleep>>) -> Result<Connection, Error> {
        timer
            .as_mut()
            .reset(Instant::now() + self.timeout_config.connect_timeout);

        let path = format!(
            "/{}{}",
            uri.authority().unwrap().as_str(),
            uri.path_and_query().unwrap().as_str()
        );

        let stream = tokio::net::UnixStream::connect(path)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::Connect)??;

        Ok(stream.into())
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
            .unwrap()
            .send()
            .await
            .unwrap()
            .body()
            .await
            .unwrap();
        println!("{}", String::from_utf8_lossy(&res));
    }
}
