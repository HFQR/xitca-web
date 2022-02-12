use xitca_io::bytes::Bytes;

use super::response::ResponseSender;

pub struct Request {
    pub(crate) tx: ResponseSender,
    pub(crate) msg: Bytes,
}
