use std::{
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};

use xitca_http::{
    bytes::{Bytes, BytesMut},
    error::BodyError,
    h1::proto::trasnder_coding::{ChunkResult, TransferCoding},
};
use xitca_io::io::Interest;

use crate::{
    body::{Body, Frame, SizeHint},
    pool::service::ExclusiveLease,
};

pub type Connection = ExclusiveLease;

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

impl Body for ResponseBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        let this = self.get_mut();

        loop {
            match this.decoder.decode(&mut this.buf) {
                ChunkResult::Ok(bytes) => return Poll::Ready(Some(Ok(Frame::Data(bytes)))),
                ChunkResult::Trailers(trailers) => return Poll::Ready(Some(Ok(Frame::Trailers(trailers)))),
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

    fn is_end_stream(&self) -> bool {
        matches!(self.decoder, TransferCoding::Eof | TransferCoding::Corrupted)
    }

    fn size_hint(&self) -> SizeHint {
        match self.decoder {
            TransferCoding::Length(size) => SizeHint::Exact(size),
            TransferCoding::Corrupted | TransferCoding::Eof => SizeHint::None,
            _ => SizeHint::Unknown,
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
