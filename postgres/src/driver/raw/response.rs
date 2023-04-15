use core::{
    future::{poll_fn, Future},
    task::{ready, Poll},
};

use postgres_protocol::message::backend;
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
