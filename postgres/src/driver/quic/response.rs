use postgres_protocol::message::backend;

use quinn::RecvStream;
use xitca_io::bytes::BytesMut;

use crate::error::{unexpected_eof_err, Error};

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
        loop {
            match backend::Message::parse(&mut self.buf)? {
                // TODO: error response.
                Some(backend::Message::ErrorResponse(_body)) => return Err(Error::todo()),
                Some(msg) => return Ok(msg),
                None => {
                    let chunk = self
                        .rx
                        .read_chunk(4096, true)
                        .await
                        .unwrap()
                        .ok_or_else(unexpected_eof_err)?;
                    self.buf.extend_from_slice(&chunk.bytes);
                }
            }
        }
    }
}
