use xitca_io::bytes::BytesMut;
use xitca_unsafe_collection::channel::spsc::channel;

use super::{response::Response, response::ResponseSender};

pub struct Request {
    pub(crate) tx: Option<ResponseSender>,
    pub(crate) msg: BytesMut,
}

impl Request {
    // a Request that does not care for a response from database.
    pub(crate) fn new(msg: BytesMut) -> Self {
        Self { tx: None, msg }
    }

    pub(crate) fn new_pair(msg: BytesMut) -> (Self, Response) {
        let (tx, rx) = channel(8);
        (Self { tx: Some(tx), msg }, Response::new(rx))
    }
}
