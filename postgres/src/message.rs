use std::collections::VecDeque;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::bytes::Bytes;

use super::{client::Client, error::Error};

pub struct Request {
    tx: UnboundedSender<()>,
    msg: Bytes,
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

pub(crate) struct RequestList {
    lst: VecDeque<Request>,
}

impl RequestList {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self {
            lst: VecDeque::with_capacity(cap),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.lst.len()
    }

    pub(crate) fn push(&mut self, req: Request) {
        self.lst.push_back(req);
    }
}
