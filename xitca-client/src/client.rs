use bytes::Bytes;
use futures_core::Stream;
use http::{Response, Uri};
use tokio::net::TcpStream;

use crate::builder::ClientBuilder;
use crate::connect::Connect;
use crate::connection::Connection;
use crate::error::Error;
use crate::pool::Pool;
use crate::resolver::Resolver;
use crate::timeout::TimeoutConfig;
use crate::tls::connector::Connector;

pub struct Client {
    pub(crate) pool: Pool<Uri, Connection>,
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

        // let conn = self.pool.acquire(uri.authority()).await?;

        // if conn.is_none() {

        // }

        let mut connect = Connect::new(uri);
        self.resolver.resolve(&mut connect).await?;

        let stream = TcpStream::connect(connect.addrs().next().unwrap()).await?;

        let stream = self.connector.connect(stream, connect.hostname()).await?;

        Ok(Response::new(Box::pin(Dummy)))
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
