use core::{
    future::poll_fn,
    task::{ready, Context, Poll},
};

use postgres_protocol::message::backend;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use xitca_io::bytes::BytesMut;

use crate::io::buffered::PagedBytesMut;

use super::error::{unexpected_eof_err, Error};

pub struct Response {
    rx: ResponseReceiver,
    buf: BytesMut,
}

pub type ResponseSender = UnboundedSender<BytesMut>;

pub type ResponseReceiver = UnboundedReceiver<BytesMut>;

impl Response {
    pub(crate) fn new(rx: UnboundedReceiver<BytesMut>) -> Self {
        Self {
            rx,
            buf: BytesMut::new(),
        }
    }

    pub(crate) async fn recv(&mut self) -> Result<backend::Message, Error> {
        poll_fn(|cx| self.poll_recv(cx)).await
    }

    pub(crate) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<backend::Message, Error>> {
        if self.buf.is_empty() {
            match ready!(self.rx.poll_recv(cx)) {
                Some(buf) => self.buf = buf,
                None => return Poll::Ready(Err(Error::from(unexpected_eof_err()))),
            }
        }

        let res = match backend::Message::parse(&mut self.buf)? {
            // TODO: error response.
            Some(backend::Message::ErrorResponse(_body)) => Err(Error::ToDo),
            Some(msg) => Ok(msg),
            None => unreachable!("must not parse message from empty buffer."),
        };
        Poll::Ready(res)
    }
}

pub enum ResponseMessage {
    Normal { buf: BytesMut, complete: bool },
    Async(backend::Message),
}

impl ResponseMessage {
    pub(crate) fn try_from_buf(buf: &mut PagedBytesMut) -> Result<Option<Self>, Error> {
        let mut idx = 0;
        let mut complete = false;

        loop {
            let slice = &buf[idx..];
            let Some(header) = backend::Header::parse(slice)? else { break };
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
                        let message = backend::Message::parse(buf.get_mut())?.unwrap();
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
