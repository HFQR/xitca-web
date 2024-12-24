use std::{
    io,
    ops::DerefMut,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use xitca_http::{
    bytes::{Bytes, BytesMut},
    error::BodyError,
    h1::proto::codec::{ChunkResult, TransferCoding},
};
use xitca_io::io::{AsyncIo, Interest};

pub struct ResponseBody<C> {
    conn: C,
    buf: BytesMut,
    decoder: TransferCoding,
}

impl<C> ResponseBody<C> {
    pub(crate) fn new(conn: C, buf: BytesMut, decoder: TransferCoding) -> Self {
        Self { conn, buf, decoder }
    }

    pub(crate) fn conn(&self) -> &C {
        &self.conn
    }

    pub(crate) fn conn_mut(&mut self) -> &mut C {
        &mut self.conn
    }
}

impl<C> Stream for ResponseBody<C>
where
    C: DerefMut + Unpin,
    C::Target: AsyncIo + Sized,
{
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match this.decoder.decode(&mut this.buf) {
                ChunkResult::Ok(bytes) => return Poll::Ready(Some(Ok(bytes))),
                ChunkResult::InsufficientData => 'inner: loop {
                    match xitca_unsafe_collection::bytes::read_buf(&mut *this.conn, &mut this.buf) {
                        Ok(n) => {
                            if n == 0 {
                                return Poll::Ready(Some(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())));
                            }
                            break 'inner;
                        }
                        Err(e) => {
                            if e.kind() != io::ErrorKind::WouldBlock {
                                return Poll::Ready(Some(Err(e.into())));
                            }

                            ready!(Pin::new(&mut **this.conn_mut()).poll_ready(Interest::READABLE, cx))?;
                        }
                    }
                },
                ChunkResult::Err(e) => return Poll::Ready(Some(Err(e.into()))),
                _ => return Poll::Ready(None),
            }
        }
    }
}
