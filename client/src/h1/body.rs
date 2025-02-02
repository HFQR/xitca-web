use std::{
    io,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use xitca_http::{
    bytes::{Bytes, BytesMut},
    error::BodyError,
    h1::proto::codec::{ChunkResult, TransferCoding},
};
use xitca_io::io::Interest;

use crate::{
    connection::{ConnectionExclusive, ConnectionKey},
    pool::exclusive::Conn,
};

pub type Connection = Conn<ConnectionKey, ConnectionExclusive>;

pub struct ResponseBody {
    conn: Connection,
    buf: BytesMut,
    decoder: TransferCoding,
}

impl ResponseBody {
    pub(crate) fn new(conn: Connection, buf: BytesMut, decoder: TransferCoding) -> Self {
        Self { conn, buf, decoder }
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

impl Stream for ResponseBody {
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

impl Drop for ResponseBody {
    fn drop(&mut self) {
        if !self.decoder.is_eof() {
            self.conn.destroy_on_drop()
        }
    }
}
