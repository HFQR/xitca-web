//! udp socket with quic protocol as client transport layer.

mod codec;
mod request;
mod response;

pub use self::{request::Request, response::Response};

use core::future::Future;

use quinn::Connection;
use xitca_io::bytes::{Bytes, BytesMut};

use crate::{client::Client, config::Config, error::Error};

use self::codec::Codec;

#[derive(Debug)]
pub(crate) struct ClientTx {
    inner: Connection,
    codec: Codec,
}

impl ClientTx {
    pub(crate) fn is_closed(&self) -> bool {
        todo!()
    }

    pub(crate) async fn send(&self, msg: BytesMut) -> Result<Response, Error> {
        let (mut tx, rx) = self.inner.open_bi().await.unwrap();
        let mut bufs = [Bytes::copy_from_slice(&msg.len().to_be_bytes()), msg.freeze()];
        let b = tx.write_chunks(&mut bufs).await.unwrap();
        // Ok(Response::new(rx))}
        todo!()
    }

    pub(crate) async fn send2(&self, msg: BytesMut) -> Result<(), Error> {
        let mut tx = self.inner.open_uni().await.unwrap();
        let mut bufs = [Bytes::copy_from_slice(&msg.len().to_be_bytes()), msg.freeze()];
        let b = tx.write_chunks(&mut bufs).await.unwrap();
        Ok(())
    }

    pub(crate) fn do_send(&self, msg: BytesMut) {}
}

pub(crate) async fn connect(cfg: Config) -> Result<(Client, impl Future<Output = Result<(), Error>> + Send), Error> {
    Ok((todo!(), async { Ok(()) }))
}
