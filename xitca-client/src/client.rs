use bytes::Bytes;
use futures_core::Stream;
use http::Response;
use tokio::net::TcpStream;

use crate::builder::ClientBuilder;
use crate::connect::Connect;
use crate::error::Error;
use crate::resolver::Resolver;
use crate::tls::connector::Connector;

pub struct Client {
    pub(crate) connector: Connector,
    pub(crate) resolver: Resolver,
}

impl Client {
    pub fn new() -> Self {
        ClientBuilder::default().finish()
    }

    pub async fn get(&self, url: &str) -> Result<Response<impl Stream<Item = Result<Bytes, Error>>>, Error> {
        let mut connect = Connect::new(url);

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
