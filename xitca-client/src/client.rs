use bytes::Bytes;
use futures_core::Stream;
use http::{uri::Authority, Response};
use tokio::net::TcpStream;

use crate::builder::ClientBuilder;
use crate::connect::Connect;
use crate::connection::Connection;
use crate::error::{Error, TimeoutError};
use crate::pool::Pool;
use crate::resolver::Resolver;
use crate::timeout::{Timeout, TimeoutConfig};
use crate::tls::connector::Connector;

pub struct Client {
    pub(crate) pool: Pool<Authority, Connection>,
    pub(crate) connector: Connector,
    pub(crate) resolver: Resolver,
    pub(crate) timeout_config: TimeoutConfig,
}

impl Client {
    pub fn new() -> Self {
        ClientBuilder::default().finish()
    }

    pub async fn get(&self, url: &str) -> Result<Response<impl Stream<Item = Result<Bytes, Error>>>, Error> {
        let uri = crate::uri::try_parse_uri(url)?;

        let key = uri.authority().unwrap().clone();

        let mut conn = self.pool.acquire(key).await?;

        // Nothing in the pool. construct new connection and add it to Conn.
        if conn.is_none() {
            let mut connect = Connect::new(uri);
            let c = self.make_connection(&mut connect).await?;
            conn.add_conn(c);
        }

        Ok(Response::new(Box::pin(Dummy)))
    }

    async fn make_connection(&self, connect: &mut Connect) -> Result<Connection, Error> {
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
