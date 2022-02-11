use std::cell::RefCell;

use tokio::sync::mpsc::Sender;
use xitca_io::bytes::BytesMut;

use super::{error::Error, message::Request};

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
