use std::{
    io,
    ops::DerefMut,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use tokio::io::{AsyncRead, ReadBuf};
use xitca_http::{
    bytes::{Bytes, BytesMut},
    error::BodyError,
    h1::proto::codec::{ChunkResult, TransferCoding},
};

pub struct ResponseBody<C> {
    conn: C,
    buf: BytesMut,
    // a chunker reader is used to provide a safe rust only api for reading into buf.
    // this is less efficient than reading directly into BytesMut.
    chunk: Vec<u8>,
    decoder: TransferCoding,
}

impl<C> ResponseBody<C> {
    pub(crate) fn new(conn: C, buf: BytesMut, chunk: Vec<u8>, decoder: TransferCoding) -> Self {
        Self {
            conn,
            buf,
            chunk,
            decoder,
        }
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
            match this.decoder.decode(&mut this.buf) {
                ChunkResult::Ok(bytes) => return Poll::Ready(Some(Ok(bytes))),
                ChunkResult::InsufficientData => {
                    let mut buf = ReadBuf::new(this.chunk.as_mut_slice());
                    ready!(Pin::new(&mut *this.conn).poll_read(cx, &mut buf))?;
                    let filled = buf.filled();

                    if filled.is_empty() {
                        return Poll::Ready(Some(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())));
                    }

                    this.buf.extend_from_slice(filled);
                }
                ChunkResult::Err(e) => return Poll::Ready(Some(Err(e.into()))),
                _ => return Poll::Ready(None),
            }
        }
    }
}
