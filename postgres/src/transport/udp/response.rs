use postgres_protocol::message::backend;

use quinn::RecvStream;
use xitca_io::bytes::BytesMut;

use crate::error::Error;

pub struct Response {
    rx: RecvStream,
    buf: BytesMut,
}

impl Response {
    pub(super) fn new(rx: RecvStream) -> Self {
        Self {
            rx,
            buf: BytesMut::new(),
        }
    }

    pub(crate) async fn recv(&mut self) -> Result<backend::Message, Error> {
        todo!()
    }
}
