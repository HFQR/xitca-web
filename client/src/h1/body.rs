use std::{
    io,
    ops::DerefMut,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use tokio::io::AsyncRead;
use tokio_util::io::poll_read_buf;
use xitca_http::{
    bytes::Bytes,
    error::BodyError,
    h1::proto::{
        buf::FlatBuf,
        codec::{ChunkResult, TransferCoding},
    },
};

pub struct ResponseBody<C> {
    conn: C,
    buf: FlatBuf<{ 1024 * 1024 }>,
    decoder: TransferCoding,
}

impl<C> ResponseBody<C> {
    pub(crate) fn new(conn: C, buf: FlatBuf<{ 1024 * 1024 }>, decoder: TransferCoding) -> Self {
        Self { conn, buf, decoder }
    }

    pub(crate) fn conn(&mut self) -> &mut C {
        &mut self.conn
    }
}

impl<C> Stream for ResponseBody<C>
where
    C: DerefMut + Unpin,
    C::Target: AsyncRead + Unpin + Sized,
{
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match this.decoder.decode(this.buf.deref_mut()) {
                ChunkResult::Ok(bytes) => return Poll::Ready(Some(Ok(bytes))),
                ChunkResult::NoSufficientData => {
                    let n = ready!(poll_read_buf(Pin::new(&mut *this.conn), cx, &mut *this.buf))?;

                    if n == 0 {
                        return Poll::Ready(Some(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())));
                    }
                }
                ChunkResult::Err(e) => return Poll::Ready(Some(Err(e.into()))),
                _ => return Poll::Ready(None),
            }
        }
    }
}
