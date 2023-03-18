//! udp socket with quic protocol as client transport layer.

mod codec;
mod response;

pub use self::response::Response;

use core::future::Future;

use quinn::Connection;
use xitca_io::bytes::BytesMut;

use crate::{client::Client, config::Config, error::Error};

#[derive(Debug)]
pub(crate) struct ClientTx {
    inner: Connection,
}

impl ClientTx {
    pub(crate) fn is_closed(&self) -> bool {
        todo!()
    }

    pub(crate) async fn send(&self, msg: BytesMut) -> Result<Response, Error> {
        let (mut tx, rx) = self.inner.open_bi().await.unwrap();
        tx.write_all(&msg).await.unwrap();
        tx.finish().await.unwrap();
        Ok(Response::new(rx))
    }

    pub(crate) async fn send2(&self, msg: BytesMut) -> Result<(), Error> {
        let mut tx = self.inner.open_uni().await.unwrap();
        tx.write_all(&msg).await.unwrap();
        tx.finish().await.unwrap();
        Ok(())
    }

    pub(crate) fn do_send(&self, msg: BytesMut) {}
}

pub(crate) async fn connect(cfg: Config) -> Result<(Client, impl Future<Output = Result<(), Error>> + Send), Error> {
    Ok((todo!(), async { Ok(()) }))
}
