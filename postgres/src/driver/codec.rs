pub(crate) mod encode;
pub(crate) mod response;

use core::{
    future::{Future, poll_fn},
    task::{Context, Poll, ready},
};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use xitca_io::bytes::BytesMut;

use crate::{
    error::{ClosedByDriver, Error},
    protocol::message::backend,
    types::BorrowToSql,
};

pub(super) fn request_pair() -> (ResponseSender, Response) {
    let (tx, rx) = unbounded_channel();
    (tx, Response { rx })
}

#[derive(Debug)]
pub struct Response {
    rx: ResponseReceiver,
}

impl Response {
    pub(crate) fn blocking_recv(&mut self) -> Result<backend::Message, Error> {
        let msg = self.rx.blocking_recv().ok_or_else(|| Error::from(ClosedByDriver))?;
        Self::parse_message(msg)
    }

    pub(crate) fn recv(&mut self) -> impl Future<Output = Result<backend::Message, Error>> + Send + '_ {
        poll_fn(|cx| self.poll_recv(cx))
    }

    pub(crate) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<backend::Message, Error>> {
        let msg = ready!(self.rx.poll_recv(cx)).ok_or_else(|| Error::from(ClosedByDriver))?;
        Poll::Ready(Self::parse_message(msg))
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

    fn parse_message(msg: backend::MessageRaw) -> Result<backend::Message, Error> {
        match msg.try_into_message()? {
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

pub(super) type ResponseSender = UnboundedSender<backend::MessageRaw>;

// TODO: remove this lint.
#[allow(dead_code)]
pub(super) type ResponseReceiver = UnboundedReceiver<backend::MessageRaw>;

pub(super) struct BytesMessage {
    pub(super) msg: backend::MessageRaw,
    pub(super) complete: bool,
}

impl BytesMessage {
    #[cold]
    #[inline(never)]
    pub(super) fn into_error(self) -> Error {
        match self.msg.try_into_message() {
            Err(e) => Error::from(e),
            Ok(backend::Message::ErrorResponse(body)) => Error::db(body.fields()),
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
        let Some(msg) = backend::MessageRaw::parse(buf)? else {
            return Ok(None);
        };

        match msg.tag() {
            backend::NOTICE_RESPONSE_TAG | backend::NOTIFICATION_RESPONSE_TAG | backend::PARAMETER_STATUS_TAG => {
                let message = msg.try_into_message()?;
                Ok(Some(ResponseMessage::Async(message)))
            }
            tag => {
                let complete = matches!(tag, backend::READY_FOR_QUERY_TAG);
                Ok(Some(ResponseMessage::Normal(BytesMessage { msg, complete })))
            }
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
