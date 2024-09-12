use core::{
    future::{poll_fn, Future},
    task::{ready, Context, Poll},
};

use postgres_protocol::message::backend;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::bytes::BytesMut;

use crate::error::{DbError, DriverDownReceiving, Error};

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

    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<backend::Message, Error>> {
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
            ready!(self.poll_try_into_row_affected(&mut rows, cx))?;
            Poll::Ready(Ok(rows))
        })
    }

    pub(crate) fn poll_try_into_row_affected(
        &mut self,
        rows: &mut u64,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error>> {
        loop {
            match ready!(self.poll_recv(cx))? {
                backend::Message::RowDescription(_) | backend::Message::DataRow(_) => {}
                backend::Message::CommandComplete(body) => {
                    *rows = body_to_affected_rows(&body)?;
                }
                backend::Message::EmptyQueryResponse => *rows = 0,
                backend::Message::ReadyForQuery(_) => return Poll::Ready(Ok(())),
                _ => return Poll::Ready(Err(Error::unexpected())),
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn try_into_ready(mut self) -> Result<(), Error> {
        loop {
            match self.recv().await? {
                backend::Message::RowDescription(_)
                | backend::Message::DataRow(_)
                | backend::Message::EmptyQueryResponse => {}
                backend::Message::ReadyForQuery(_) => return Ok(()),
                _ => return Err(Error::unexpected()),
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
