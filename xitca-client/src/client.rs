use std::pin::Pin;

use http::uri;
use tokio::{
    net::TcpStream,
    time::{Instant, Sleep},
};

use crate::{
    builder::ClientBuilder,
    connect::Connect,
    connection::{Connection, ConnectionKey},
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
}

impl Client {
    pub fn new() -> Self {
        ClientBuilder::default().finish()
    }

    pub fn request<B>(&self, req: http::Request<B>) -> Request<'_, B> {
        Request::new(req, self)
    }

    pub fn get(&self, url: &str) -> Result<Request<'_, ()>, Error> {
        let uri = uri::Uri::try_from(url)?;

        let mut req = http::Request::new(());
        *req.uri_mut() = uri;

        Ok(self.request(req))
    }

    pub(crate) async fn make_connection(
        &self,
        connect: &mut Connect,
        mut timer: Pin<&mut Sleep>,
    ) -> Result<Connection, Error> {
        match connect.uri {
            Uri::Tcp(_) => {
                let stream = self.make_tcp(connect, timer.as_mut()).await?;
                Ok(stream.into())
            }
            Uri::Tls(_) => {
                let stream = self.make_tcp(connect, timer.as_mut()).await?;

                timer
                    .as_mut()
                    .reset(Instant::now() + self.timeout_config.tls_connect_timeout);
                let stream = self
                    .connector
                    .connect(stream, connect.hostname())
                    .timeout(timer)
                    .await
                    .map_err(|_| TimeoutError::TlsHandshake)??;

                Ok(stream.into())
            }
            #[cfg(unix)]
            Uri::Unix(ref uri) => {
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
    }

    async fn make_tcp(&self, connect: &mut Connect, mut timer: Pin<&mut Sleep>) -> Result<TcpStream, Error> {
        self.resolver
            .resolve(connect)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::Resolve)??;

        timer
            .as_mut()
            .reset(Instant::now() + self.timeout_config.connect_timeout);
        let stream = TcpStream::connect(connect.addrs().next().unwrap())
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::Connect)??;

        Ok(stream)
    }
}
