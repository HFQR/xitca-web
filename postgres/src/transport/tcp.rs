//! tcp socket client.

mod io;
mod request;
mod response;

pub use self::{io::BufferedIo, request::Request, response::Response};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::bytes::BytesMut;

type ResponseSender = UnboundedSender<BytesMut>;

type ResponseReceiver = UnboundedReceiver<BytesMut>;

pub(crate) fn new_pair(msg: BytesMut) -> (Request, Response) {
    let (tx, rx) = unbounded_channel();
    (Request::new(Some(tx), msg), Response::new(rx))
}

pub type ClientTx = UnboundedSender<Request>;
