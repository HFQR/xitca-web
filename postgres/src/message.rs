use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::bytes::Bytes;

use super::{client::Client, error::Error};

pub struct Request {
    pub(crate) tx: UnboundedSender<()>,
    pub(crate) msg: Bytes,
}

pub struct Response {
    rx: UnboundedReceiver<()>,
}

impl Client {
    pub(crate) async fn send(&self, msg: Bytes) -> Result<Response, Error> {
        let (tx, rx) = unbounded_channel();

        self.tx
            .send(Request { tx, msg })
            .await
            .map_err(|_| Error::ConnectionClosed)?;

        Ok(Response { rx })
    }
}
