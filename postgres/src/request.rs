use xitca_io::bytes::Bytes;
use xitca_unsafe_collection::channel::spsc::channel;

use super::{response::Response, response::ResponseSender};

pub struct Request {
    pub(crate) tx: ResponseSender,
    pub(crate) msg: Bytes,
}

impl Request {
    pub(crate) fn new_pair(msg: Bytes) -> (Request, Response) {
        let (tx, rx) = channel(8);

        (Request { tx, msg }, Response::new(rx))
    }
}
