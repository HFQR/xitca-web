use core::{
    future::{poll_fn, Future},
    task::{ready, Context, Poll},
};

use postgres_protocol::message::{backend, frontend};
use postgres_types::{BorrowToSql, IsNull};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::bytes::BytesMut;

use crate::{
    error::InvalidParamCount,
    error::{DbError, DriverDownReceiving, Error},
    statement::Statement,
};

use super::DriverTx;

pub(super) fn request_pair(msg_count: usize) -> (ResponseSender, Response) {
    let (tx, rx) = unbounded_channel();
    (
        ResponseSender { tx, msg_count },
        Response {
            rx,
            buf: BytesMut::new(),
        },
    )
}

pub struct Response {
    rx: ResponseReceiver,
    buf: BytesMut,
}

impl Response {
    pub(crate) fn recv(&mut self) -> impl Future<Output = Result<backend::Message, Error>> + Send + '_ {
        poll_fn(|cx| self.poll_recv(cx))
    }

    pub(crate) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<backend::Message, Error>> {
        if self.buf.is_empty() {
            self.buf = ready!(self.rx.poll_recv(cx)).ok_or_else(|| DriverDownReceiving)?;
        }

        let res = match backend::Message::parse(&mut self.buf)?.expect("must not parse message from empty buffer.") {
            backend::Message::ErrorResponse(body) => {
                let e = DbError::parse(&mut body.fields())?;
                Err(e.into())
            }
            msg => Ok(msg),
        };

        Poll::Ready(res)
    }

    pub(crate) fn try_into_row_affected(mut self) -> impl Future<Output = Result<u64, Error>> + Send {
        let mut rows = 0;
        poll_fn(move |cx| {
            ready!(self.poll_try_into_ready(&mut rows, cx))?;
            Poll::Ready(Ok(rows))
        })
    }

    pub(crate) fn poll_try_into_ready(&mut self, rows: &mut u64, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        loop {
            match ready!(self.poll_recv(cx))? {
                backend::Message::BindComplete
                | backend::Message::DataRow(_)
                | backend::Message::EmptyQueryResponse => {}
                backend::Message::CommandComplete(body) => {
                    *rows = body_to_affected_rows(&body)?;
                }
                backend::Message::ReadyForQuery(_) => return Poll::Ready(Ok(())),
                _ => return Poll::Ready(Err(Error::unexpected())),
            }
        }
    }
}

// Extract the number of rows affected.
pub(crate) fn body_to_affected_rows(body: &backend::CommandCompleteBody) -> Result<u64, Error> {
    body.tag()
        .map_err(|_| Error::todo())
        .map(|r| r.rsplit(' ').next().unwrap().parse().unwrap_or(0))
}

#[derive(Debug)]
pub(crate) struct ResponseSender {
    tx: UnboundedSender<BytesMut>,
    msg_count: usize,
}

pub(super) enum SenderState {
    Continue,
    Finish,
}

impl ResponseSender {
    pub(super) fn send(&mut self, msg: BytesMut, complete: bool) -> SenderState {
        debug_assert!(self.msg_count > 0);

        let _ = self.tx.send(msg);

        if complete {
            self.msg_count -= 1;
        }

        if self.msg_count == 0 {
            SenderState::Finish
        } else {
            SenderState::Continue
        }
    }
}

// TODO: remove this lint.
#[allow(dead_code)]
pub(super) type ResponseReceiver = UnboundedReceiver<BytesMut>;

pub enum ResponseMessage {
    Normal { buf: BytesMut, complete: bool },
    Async(backend::Message),
}

impl ResponseMessage {
    pub(crate) fn try_from_buf(buf: &mut BytesMut) -> Result<Option<Self>, Error> {
        let mut idx = 0;
        let mut complete = false;

        loop {
            let slice = &buf[idx..];
            let Some(header) = backend::Header::parse(slice)? else {
                break;
            };
            let len = header.len() as usize + 1;

            if slice.len() < len {
                break;
            }

            match header.tag() {
                backend::NOTICE_RESPONSE_TAG | backend::NOTIFICATION_RESPONSE_TAG | backend::PARAMETER_STATUS_TAG => {
                    if idx == 0 {
                        // TODO:
                        // PagedBytesMut should never expose underlying BytesMut type as reference.
                        // this is needed because postgres-protocol is an external crate.
                        let message = backend::Message::parse(buf)?.unwrap();
                        return Ok(Some(ResponseMessage::Async(message)));
                    }

                    break;
                }
                tag => {
                    idx += len;
                    if matches!(tag, backend::READY_FOR_QUERY_TAG) {
                        complete = true;
                        break;
                    }
                }
            }
        }

        if idx == 0 {
            Ok(None)
        } else {
            Ok(Some(ResponseMessage::Normal {
                buf: buf.split_to(idx),
                complete,
            }))
        }
    }
}

/// super trait to constraint Self and associated types' trait bounds.
pub trait AsParams: IntoIterator<IntoIter: ExactSizeIterator, Item: BorrowToSql> {}

impl<I> AsParams for I
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: BorrowToSql,
{
}

pub(crate) fn send_encode_query<I>(tx: &DriverTx, stmt: &Statement, params: I) -> Result<Response, Error>
where
    I: AsParams,
{
    tx.send(|buf| encode_query_maybe_sync::<_, true>(buf, stmt, params.into_iter()))
}

pub(crate) fn send_encode_query_simple(tx: &DriverTx, stmt: &str) -> Result<Response, Error> {
    tx.send(|buf| frontend::query(stmt, buf).map_err(Into::into))
}

pub(crate) fn encode_query_maybe_sync<I, const SYNC_MODE: bool>(
    buf: &mut BytesMut,
    stmt: &Statement,
    params: I,
) -> Result<(), Error>
where
    I: ExactSizeIterator,
    I::Item: BorrowToSql,
{
    encode_bind(stmt, params, "", buf)?;
    frontend::execute("", 0, buf)?;
    if SYNC_MODE {
        frontend::sync(buf);
    }
    Ok(())
}

pub(crate) fn send_encode_portal_create<I>(
    tx: &DriverTx,
    name: &str,
    stmt: &Statement,
    params: I,
) -> Result<Response, Error>
where
    I: ExactSizeIterator,
    I::Item: BorrowToSql,
{
    tx.send(|buf| {
        encode_bind(stmt, params, name, buf)?;
        frontend::sync(buf);
        Ok(())
    })
}

pub(crate) fn send_encode_portal_query(tx: &DriverTx, name: &str, max_rows: i32) -> Result<Response, Error> {
    tx.send(|buf| {
        frontend::execute(name, max_rows, buf)?;
        frontend::sync(buf);
        Ok(())
    })
}

pub(crate) fn send_encode_portal_cancel(tx: &DriverTx, name: &str) -> Result<Response, Error> {
    send_cancel(b'P', tx, name)
}

pub(crate) fn send_encode_statement_cancel(tx: &DriverTx, name: &str) -> Result<Response, Error> {
    send_cancel(b'S', tx, name)
}

fn send_cancel(variant: u8, tx: &DriverTx, name: &str) -> Result<Response, Error> {
    tx.send(|buf| {
        frontend::close(variant, name, buf)?;
        frontend::sync(buf);
        Ok(())
    })
}

fn encode_bind<I>(stmt: &Statement, params: I, portal: &str, buf: &mut BytesMut) -> Result<(), Error>
where
    I: ExactSizeIterator,
    I::Item: BorrowToSql,
{
    if params.len() != stmt.params().len() {
        return Err(Error::from(InvalidParamCount {
            expected: params.len(),
            params: stmt.params().len(),
        }));
    }

    let params = params.zip(stmt.params()).collect::<Vec<_>>();

    frontend::bind(
        portal,
        stmt.name(),
        params.iter().map(|(p, ty)| p.borrow_to_sql().encode_format(ty) as _),
        params.iter(),
        |(param, ty), buf| {
            param
                .borrow_to_sql()
                .to_sql_checked(ty, buf)
                .map(|is_null| match is_null {
                    IsNull::No => postgres_protocol::IsNull::No,
                    IsNull::Yes => postgres_protocol::IsNull::Yes,
                })
        },
        Some(1),
        buf,
    )
    .map_err(|e| match e {
        frontend::BindError::Conversion(e) => Error::from(e),
        frontend::BindError::Serialization(e) => Error::from(e),
    })
}
