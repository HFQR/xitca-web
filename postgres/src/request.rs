use crate::response::Response;
use tokio::sync::mpsc::unbounded_channel;
use xitca_io::bytes::Bytes;

use super::response::ResponseSender;

pub struct Request {
    pub(crate) tx: ResponseSender,
    pub(crate) msg: Bytes,
}

impl Request {
    pub(crate) fn new_pair(msg: Bytes) -> (Request, Response) {
        let (tx, rx) = unbounded_channel();

        (Request { tx, msg }, Response::new(rx))
    }
}
