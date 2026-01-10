pub(crate) mod encode;
pub(crate) mod response;

use core::{
    future::{Future, poll_fn},
    task::{Context, Poll, ready},
};

use postgres_protocol::message::backend;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use xitca_io::bytes::BytesMut;

use crate::{
    error::{ClosedByDriver, Error},
    types::BorrowToSql,
};

pub(super) fn request_pair() -> (ResponseSender, Response) {
    let (tx, rx) = unbounded_channel();
    (
        tx,
        Response {
            rx,
            buf: BytesMut::new(),
        },
    )
}

#[derive(Debug)]
pub struct Response {
    rx: ResponseReceiver,
    buf: BytesMut,
}

impl Response {
    pub(crate) fn blocking_recv(&mut self) -> Result<backend::Message, Error> {
        if self.buf.is_empty() {
            self.buf = self.rx.blocking_recv().ok_or_else(|| Error::from(ClosedByDriver))?;
        }
        self.parse_message()
    }

    pub(crate) fn recv(&mut self) -> impl Future<Output = Result<backend::Message, Error>> + Send + '_ {
        poll_fn(|cx| self.poll_recv(cx))
    }

    pub(crate) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<backend::Message, Error>> {
        if self.buf.is_empty() {
            self.buf = ready!(self.rx.poll_recv(cx)).ok_or_else(|| Error::from(ClosedByDriver))?;
        }
        Poll::Ready(self.parse_message())
    }

    pub(crate) fn try_into_row_affected(mut self) -> impl Future<Output = Result<u64, Error>> + Send {
        let mut rows = 0;
        poll_fn(move |cx| {
            ready!(self.poll_try_into_ready(&mut rows, cx))?;
            Poll::Ready(Ok(rows))
        })
    }

    pub(crate) fn try_into_row_affected_blocking(mut self) -> Result<u64, Error> {
        let mut rows = 0;
        loop {
            match self.blocking_recv()? {
                backend::Message::BindComplete
                | backend::Message::NoData
                | backend::Message::ParseComplete
                | backend::Message::ParameterDescription(_)
                | backend::Message::RowDescription(_)
                | backend::Message::DataRow(_)
                | backend::Message::EmptyQueryResponse
                | backend::Message::PortalSuspended => {}
                backend::Message::CommandComplete(body) => {
                    rows = body_to_affected_rows(&body)?;
                }
                backend::Message::ReadyForQuery(_) => return Ok(rows),
                _ => return Err(Error::unexpected()),
            }
        }
    }

    pub(crate) fn poll_try_into_ready(&mut self, rows: &mut u64, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        loop {
            match ready!(self.poll_recv(cx))? {
                backend::Message::BindComplete
                | backend::Message::NoData
                | backend::Message::ParseComplete
                | backend::Message::ParameterDescription(_)
                | backend::Message::RowDescription(_)
                | backend::Message::DataRow(_)
                | backend::Message::EmptyQueryResponse
                | backend::Message::PortalSuspended => {}
                backend::Message::CommandComplete(body) => {
                    *rows = body_to_affected_rows(&body)?;
                }
                backend::Message::ReadyForQuery(_) => return Poll::Ready(Ok(())),
                _ => return Poll::Ready(Err(Error::unexpected())),
            }
        }
    }

    fn parse_message(&mut self) -> Result<backend::Message, Error> {
        match backend::Message::parse(&mut self.buf)?.expect("must not parse message from empty buffer.") {
            backend::Message::ErrorResponse(body) => Err(Error::db(body.fields())),
            msg => Ok(msg),
        }
    }
}

// Extract the number of rows affected.
pub(crate) fn body_to_affected_rows(body: &backend::CommandCompleteBody) -> Result<u64, Error> {
    body.tag()
        .map_err(|_| Error::todo())
        .map(|r| r.rsplit(' ').next().unwrap().parse().unwrap_or(0))
}

pub(super) type ResponseSender = UnboundedSender<BytesMut>;

// TODO: remove this lint.
#[allow(dead_code)]
pub(super) type ResponseReceiver = UnboundedReceiver<BytesMut>;

pub(super) struct BytesMessage {
    pub(super) buf: BytesMut,
    pub(super) complete: bool,
}

impl BytesMessage {
    #[cold]
    #[inline(never)]
    pub(super) fn parse_error(&mut self) -> Error {
        match backend::Message::parse(&mut self.buf) {
            Err(e) => Error::from(e),
            Ok(Some(backend::Message::ErrorResponse(body))) => Error::db(body.fields()),
            _ => Error::unexpected(),
        }
    }
}

pub(super) enum ResponseMessage {
    Normal(BytesMessage),
    Async(backend::Message),
}

impl ResponseMessage {
    pub(crate) fn try_from_buf(buf: &mut BytesMut) -> Result<Option<Self>, Error> {
        let mut tail = 0;
        let mut complete = false;

        loop {
            let slice = &buf[tail..];
            let Some(header) = backend::Header::parse(slice)? else {
                break;
            };
            let len = header.len() as usize + 1;

            if slice.len() < len {
                break;
            }

            match header.tag() {
                backend::NOTICE_RESPONSE_TAG | backend::NOTIFICATION_RESPONSE_TAG | backend::PARAMETER_STATUS_TAG => {
                    if tail > 0 {
                        break;
                    }
                    let message = backend::Message::parse(buf)?
                        .expect("buffer contains at least one Message. parser must produce Some");
                    return Ok(Some(ResponseMessage::Async(message)));
                }
                tag => {
                    tail += len;
                    if matches!(tag, backend::READY_FOR_QUERY_TAG) {
                        complete = true;
                        break;
                    }
                }
            }
        }

        if tail == 0 {
            Ok(None)
        } else {
            Ok(Some(ResponseMessage::Normal(BytesMessage {
                buf: buf.split_to(tail),
                complete,
            })))
        }
    }
}

/// traits for converting typed parameters into exact sized iterator where it yields
/// item can be converted in binary format of postgres type.
pub trait AsParams: IntoIterator<IntoIter: ExactSizeIterator<Item: BorrowToSql> + Clone> {}

impl<I> AsParams for I where I: IntoIterator<IntoIter: ExactSizeIterator<Item: BorrowToSql> + Clone> {}

mod sealed {
    pub trait Sealed {}
}
