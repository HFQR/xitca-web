use core::{
    future::poll_fn,
    task::{ready, Context, Poll},
};

use postgres_protocol::message::backend;
use xitca_io::bytes::BytesMut;

use crate::{
    error::{unexpected_eof_err, Error},
    transport::codec::ResponseReceiver,
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

    pub(crate) async fn recv(&mut self) -> Result<backend::Message, Error> {
        poll_fn(|cx| self.poll_recv(cx)).await
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<backend::Message, Error>> {
        if self.buf.is_empty() {
            self.buf = ready!(self.rx.poll_recv(cx)).ok_or_else(unexpected_eof_err)?;
        }

        let res = match backend::Message::parse(&mut self.buf)?.expect("must not parse message from empty buffer.") {
            // TODO: error response.
            backend::Message::ErrorResponse(_body) => Err(Error::ToDo),
            msg => Ok(msg),
        };

        Poll::Ready(res)
    }
}
