use std::cell::RefCell;

use tokio::sync::mpsc::{unbounded_channel, Sender};
use xitca_io::bytes::{Bytes, BytesMut};

use super::{error::Error, request::Request, response::Response};

#[derive(Debug)]
pub struct Client {
    pub(crate) tx: Sender<Request>,
    pub(crate) buf: RefCell<BytesMut>,
}

impl Client {
    pub(crate) fn new(tx: Sender<Request>) -> Self {
        Self {
            tx,
            buf: RefCell::new(BytesMut::new()),
        }
    }

    pub async fn query(&self) -> Result<(), Error> {
        todo!()
    }

    pub fn closed(&self) -> bool {
        self.tx.is_closed()
    }

    pub(crate) async fn send(&self, msg: Bytes) -> Result<Response, Error> {
        let (tx, rx) = unbounded_channel();

        self.tx
            .send(Request { tx, msg })
            .await
            .map_err(|_| Error::ConnectionClosed)?;

        Ok(Response::new(rx))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn assert_send<C: Send>() {}

    #[test]
    fn is_send() {
        assert_send::<Client>();
    }
}
