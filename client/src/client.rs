use std::{net::SocketAddr, pin::Pin};

use futures_core::stream::Stream;
use tokio::{
    net::{TcpSocket, TcpStream},
    time::{Instant, Sleep},
};

use crate::{
    body::{BodyError, Empty},
    builder::ClientBuilder,
    bytes::Bytes,
    connect::Connect,
    connection::{Connection, ConnectionKey},
    date::DateTimeService,
    error::{Error, TimeoutError},
    http::{self, uri, Method, Version},
    pool::Pool,
    request::Request,
    resolver::Resolver,
    timeout::{Timeout, TimeoutConfig},
    tls::connector::Connector,
    uri::Uri,
};

pub struct Client {
    pub(crate) pool: Pool<ConnectionKey, Connection>,
    pub(crate) connector: Connector,
    pub(crate) resolver: Resolver,
    pub(crate) timeout_config: TimeoutConfig,
    pub(crate) local_addr: Option<SocketAddr>,
    pub(crate) date_service: DateTimeService,
    #[cfg(feature = "http3")]
    pub(crate) h3_client: h3_quinn::quinn::Endpoint,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// Construct a new Client with default setting.
    pub fn new() -> Self {
        Self::builder().finish()
    }

    /// Start a new ClientBuilder.
    ///
    /// See [ClientBuilder] for detail.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Start a new HTTP request with given [http::Request].
    #[inline]
    pub fn request<B, E>(&self, req: http::Request<B>) -> Request<'_, B>
    where
        B: Stream<Item = Result<Bytes, E>>,
        BodyError: From<E>,
    {
        Request::new(req, self)
    }

    /// Start a new GET request with empty request body.
    pub fn get<U>(&self, url: U) -> Result<Request<'_, Empty<Bytes>>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        let uri = uri::Uri::try_from(url)?;

        let mut req = http::Request::new(Default::default());
        *req.uri_mut() = uri;

        Ok(self.request(req))
    }

    /// Start a new POST request with empty request body.
    #[inline]
    pub fn post<U>(&self, url: U) -> Result<Request<'_, Empty<Bytes>>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        Ok(self.get(url)?.method(Method::POST))
    }

    /// Start a new PUT request with empty request body.
    #[inline]
    pub fn put<U>(&self, url: U) -> Result<Request<'_, Empty<Bytes>>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        Ok(self.get(url)?.method(Method::PUT))
    }

    /// Start a new PATCH request with empty request body.
    #[inline]
    pub fn patch<U>(&self, url: U) -> Result<Request<'_, Empty<Bytes>>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        Ok(self.get(url)?.method(Method::PATCH))
    }

    /// Start a new DELETE request with empty request body.
    #[inline]
    pub fn delete<U>(&self, url: U) -> Result<Request<'_, Empty<Bytes>>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        Ok(self.get(url)?.method(Method::DELETE))
    }

    /// Start a new OPTIONS request with empty request body.
    #[inline]
    pub fn options<U>(&self, url: U) -> Result<Request<'_, Empty<Bytes>>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        Ok(self.get(url)?.method(Method::OPTIONS))
    }

    /// Start a new HEAD request with empty request body.
    #[inline]
    pub fn head<U>(&self, url: U) -> Result<Request<'_, Empty<Bytes>>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        Ok(self.get(url)?.method(Method::HEAD))
    }

    /// Start a new CONNECT request with empty request body.
    #[inline]
    pub fn connect<U>(&self, url: U) -> Result<Request<'_, Empty<Bytes>>, Error>
    where
        uri::Uri: TryFrom<U>,
        Error: From<<uri::Uri as TryFrom<U>>::Error>,
    {
        Ok(self.get(url)?.method(Method::CONNECT))
    }

    #[cfg(feature = "websocket")]
    /// Start a new websocket request.
    pub fn ws(&self, url: &str) -> Result<Request<'_, Empty<Bytes>>, Error> {
        let req = http_ws::client_request_from_uri(url)?.map(|_| Default::default());
        Ok(Request::new(req, self))
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
                    .resolve(connect)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| TimeoutError::Resolve)??;

                self.make_tcp(connect, timer).await.map(Into::into)
            }
            Uri::Tls(_) => {
                self.resolver
                    .resolve(connect)
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

    async fn make_tcp(&self, connect: &mut Connect<'_>, timer: &mut Pin<Box<Sleep>>) -> Result<TcpStream, Error> {
        timer
            .as_mut()
            .reset(Instant::now() + self.timeout_config.connect_timeout);

        let stream = self
            .make_tcp_inner(connect)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::Connect)??;

        Ok(stream)
    }

    async fn make_tcp_inner(&self, connect: &mut Connect<'_>) -> Result<TcpStream, Error> {
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
        connect: &mut Connect<'_>,
        timer: &mut Pin<Box<Sleep>>,
        max_version: Version,
    ) -> Result<Connection, Error> {
        let stream = self.make_tcp(connect, timer).await?;

        timer
            .as_mut()
            .reset(Instant::now() + self.timeout_config.tls_connect_timeout);

        let (stream, version) = self
            .connector
            .connect(stream, connect.hostname())
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
    async fn make_h3(&self, connect: &mut Connect<'_>, timer: &mut Pin<Box<Sleep>>) -> Result<Connection, Error> {
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
    async fn make_h3_inner(&self, connect: &mut Connect<'_>) -> Result<Connection, Error> {
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
