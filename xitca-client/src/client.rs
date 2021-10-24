use bytes::Bytes;
use futures_core::Stream;
use http::uri;
use tokio::net::TcpStream;

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

    pub(crate) async fn make_connection(&self, connect: &mut Connect) -> Result<Connection, Error> {
        let timer = tokio::time::sleep(self.timeout_config.resolve_timeout);
        tokio::pin!(timer);

        self.resolver
            .resolve(connect)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::Resolve)??;

        use tokio::time::Instant;

        timer
            .as_mut()
            .reset(Instant::now() + self.timeout_config.connect_timeout);
        let stream = TcpStream::connect(connect.addrs().next().unwrap())
            .timeout(timer.as_mut())
            .await
            .map_err(|_| TimeoutError::Connect)??;

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
}

struct Dummy;

impl Stream for Dummy {
    type Item = Result<Bytes, Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        todo!()
    }
}
