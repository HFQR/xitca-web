use core::{
    future::{poll_fn, Future},
    task::{ready, Poll},
};

use postgres_protocol::message::backend;
use tokio::sync::mpsc::unbounded_channel;
use xitca_io::bytes::BytesMut;

use crate::{
    driver::codec::ResponseReceiver,
    error::{unexpected_eof_err, Error},
};

pub struct Response {
    rx: ResponseReceiver,
    buf: BytesMut,
}

impl Response {
    // a no-op response for empty streaming response. this is a hack to avoid adding new error
    // variant for case where user providing an empty query.
    pub(crate) fn no_op() -> Self {
        let (_, rx) = unbounded_channel();
        Self::new(rx)
    }

    pub(crate) fn new(rx: ResponseReceiver) -> Self {
        Self {
            rx,
            buf: BytesMut::new(),
        }
    }

    pub(crate) fn recv(&mut self) -> impl Future<Output = Result<backend::Message, Error>> + '_ {
        poll_fn(|cx| {
            if self.buf.is_empty() {
                self.buf = ready!(self.rx.poll_recv(cx)).ok_or_else(unexpected_eof_err)?;
            }

            let res = match backend::Message::parse(&mut self.buf)?.expect("must not parse message from empty buffer.")
            {
                // TODO: error response.
                backend::Message::ErrorResponse(_body) => Err(Error::ToDo),
                msg => Ok(msg),
            };

            Poll::Ready(res)
        })
    }
}
