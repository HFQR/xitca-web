use postgres_protocol::message::backend;

use quinn::RecvStream;
use xitca_io::bytes::BytesMut;

use crate::error::{unexpected_eof_err, Error};

use super::codec::decode;

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
            if let Some(msg) = decode(&mut self.buf)? {
                return Ok(msg);
            }

            let chunk = self
                .rx
                .read_chunk(usize::MAX, true)
                .await
                .unwrap()
                .ok_or_else(|| unexpected_eof_err())?;

            self.buf.extend_from_slice(&chunk.bytes);
        }
    }
}
