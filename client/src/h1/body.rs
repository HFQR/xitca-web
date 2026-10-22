use std::{
    io,
    ops::{Deref, DerefMut},
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
    connection::ConnectionExclusive,
    pool::service::ExclusiveLease,
};

/// The connection backing an H1 response body.
///
/// `Pooled` holds the full pool lease (including the capacity semaphore permit).
/// `Detached` holds only the raw TCP/TLS connection; the pool slot was already
/// released. Use [`ResponseBody::detach_from_pool`] to transition from the
/// former to the latter for long-lived responses such as Server-Sent Events.
pub(crate) enum Connection {
    Pooled(ExclusiveLease),
    Detached(ConnectionExclusive),
}

impl Deref for Connection {
    type Target = ConnectionExclusive;

    fn deref(&self) -> &ConnectionExclusive {
        match self {
            Connection::Pooled(lease) => lease.deref(),
            Connection::Detached(conn) => conn,
        }
    }
}

impl DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut ConnectionExclusive {
        match self {
            Connection::Pooled(lease) => lease.deref_mut(),
            Connection::Detached(conn) => conn,
        }
    }
}

impl Connection {
    fn mark_destroy(&mut self) {
        if let Connection::Pooled(lease) = self {
            lease.mark_destroy();
        }
    }

    pub fn detach(&mut self) {
        let conn = match self {
            Connection::Pooled(lease) => lease.detach(),
            Connection::Detached(_) => return,
        };

        *self = Connection::Detached(conn)
    }
}

pub struct ResponseBody {
    conn: Connection,
    buf: BytesMut,
    decoder: TransferCoding,
}

impl ResponseBody {
    pub(crate) fn new(conn: ExclusiveLease, buf: BytesMut, decoder: TransferCoding) -> Self {
        Self {
            conn: Connection::Pooled(conn),
            buf,
            decoder,
        }
    }

    fn io_mut(&mut self) -> &mut ConnectionExclusive {
        &mut self.conn
    }

    pub(crate) fn conn(&self) -> &ConnectionExclusive {
        &self.conn
    }

    pub(crate) fn conn_mut(&mut self) -> &mut ConnectionExclusive {
        self.io_mut()
    }

    pub(crate) fn mark_destroy(&mut self) {
        self.conn.mark_destroy();
    }

    pub(crate) fn is_marked_destroy(&self) -> bool {
        match &self.conn {
            Connection::Pooled(lease) => lease.is_marked_destroy(),
            // Detached connections are never returned to the pool.
            Connection::Detached(_) => true,
        }
    }

    pub(crate) fn detach_from_pool(&mut self) {
        self.conn.detach();
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
                    match xitca_unsafe_collection::bytes::read_buf(this.conn.deref_mut(), &mut this.buf) {
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

                            ready!(Pin::new(this.conn_mut().deref_mut()).poll_ready(Interest::READABLE, cx))?;
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
            self.conn.mark_destroy()
        }
    }
}
