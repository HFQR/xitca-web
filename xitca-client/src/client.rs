use std::{net::SocketAddr, pin::Pin};

use tokio::{
    net::{TcpSocket, TcpStream},
    time::{Instant, Sleep},
};
use xitca_http::http::{self, uri, Method, Version};

use crate::{
    body::RequestBody,
    builder::ClientBuilder,
    connect::Connect,
    connection::{Connection, ConnectionKey},
    date::DateTimeService,
    error::{Error, TimeoutError},
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
    pub fn request<B>(&self, req: http::Request<RequestBody<B>>) -> Request<'_, B> {
        Request::new(req, self)
    }

    /// Start a new GET request with empty request body.
    pub fn get(&self, url: &str) -> Result<Request<'_, RequestBody>, Error> {
        let uri = uri::Uri::try_from(url)?;

        let mut req = http::Request::new(RequestBody::None);
        *req.uri_mut() = uri;

        Ok(self.request(req))
    }

    /// Start a new POST request with empty request body.
    pub fn post(&self, url: &str) -> Result<Request<'_, RequestBody>, Error> {
        let uri = uri::Uri::try_from(url)?;

        let mut req = http::Request::new(RequestBody::None);
        *req.uri_mut() = uri;
        *req.method_mut() = Method::POST;

        Ok(self.request(req))
    }
}

impl Client {
    pub(crate) async fn make_connection(
        &self,
        connect: &mut Connect<'_>,
        timer: &mut Pin<Box<Sleep>>,
    ) -> Result<Connection, Error> {
        match connect.uri {
            Uri::Tcp(_) => self.make_tcp(connect, timer).await.map(Into::into),
            Uri::Tls(_) => self.make_tls(connect, timer).await,
            #[cfg(unix)]
            Uri::Unix(uri) => self.make_unix(uri, timer).await,
        }
    }

    async fn make_tcp(&self, connect: &mut Connect<'_>, timer: &mut Pin<Box<Sleep>>) -> Result<TcpStream, Error> {
        self.resolver
            .resolve(connect)
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

    async fn make_tls(&self, connect: &mut Connect<'_>, timer: &mut Pin<Box<Sleep>>) -> Result<Connection, Error> {
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

        match version {
            Version::HTTP_11 => Ok(stream.into()),
            Version::HTTP_2 => {
                #[cfg(feature = "http2")]
                {
                    let (handle, conn) = h2::client::handshake(stream).await?;
                    tokio::spawn(async {
                        conn.await.expect("http2 connection failed");
                    });

                    Ok(handle.into())
                }

                #[cfg(not(feature = "http2"))]
                {
                    Ok(stream.into())
                }
            }
            Version::HTTP_3 => unimplemented!("Protocol::HTTP3 is not yet supported"),
            _ => unreachable!("HTTP_09 and HTTP_10 is impossible for tls connections"),
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
