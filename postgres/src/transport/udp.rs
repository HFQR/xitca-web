//! udp socket with quic protocol as client transport layer.

mod codec;
mod response;

pub use self::response::Response;

use core::future::Future;

use quinn::{Connection, SendStream};
use xitca_io::bytes::{Buf, Bytes, BytesMut};

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
        let mut buf = self.codec.encode(msg);
        let (tx, rx) = self.inner.open_bi().await.unwrap();
        _send(tx, &mut buf).await?;
        Ok(Response::new(rx))
    }

    pub(crate) async fn send2(&self, msg: BytesMut) -> Result<(), Error> {
        let mut buf = self.codec.encode(msg);
        let tx = self.inner.open_uni().await.unwrap();
        _send(tx, &mut buf).await
    }

    pub(crate) fn do_send(&self, msg: BytesMut) {}
}

async fn _send(mut tx: SendStream, mut bufs: &mut [Bytes]) -> Result<(), Error> {
    loop {
        let written = tx.write_chunks(bufs).await.unwrap();

        // this logic depend on quinn's write_chunks behavior:
        // Written.chunks should always represent the count of Bytes that are fully written.
        let (done, rest) = bufs.split_at_mut(written.chunks);

        if rest.is_empty() {
            break;
        }

        // adjust the length of slice and split the first Bytes that is (possibly) not fully
        // written.
        let chunks_len = done.iter().map(|buf| buf.len()).sum::<usize>();
        bufs = rest;
        bufs[0].advance(written.bytes - chunks_len);
    }
    tx.finish().await.unwrap();
    Ok(())
}

pub(crate) async fn connect(cfg: Config) -> Result<(Client, impl Future<Output = Result<(), Error>> + Send), Error> {
    Ok((todo!(), async { Ok(()) }))
}
