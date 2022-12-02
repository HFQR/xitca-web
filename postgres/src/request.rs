use xitca_io::bytes::BytesMut;
use xitca_unsafe_collection::channel::spsc::channel;

use super::{response::Response, response::ResponseSender};

pub struct Request {
    pub(crate) tx: Option<ResponseSender>,
    pub(crate) msg: BytesMut,
}

impl Request {
    pub(crate) fn new_pair(msg: BytesMut) -> (Request, Response) {
        let (tx, rx) = channel(8);

        (Request { tx: Some(tx), msg }, Response::new(rx))
    }
}
