use xitca_io::bytes::BytesMut;

use super::ResponseSender;

pub struct Request {
    pub(crate) tx: Option<ResponseSender>,
    pub(crate) msg: BytesMut,
}

impl Request {
    // a Request that does not care for a response from database.
    pub(crate) fn new(tx: Option<ResponseSender>, msg: BytesMut) -> Self {
        Self { tx, msg }
    }
}
