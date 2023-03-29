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
