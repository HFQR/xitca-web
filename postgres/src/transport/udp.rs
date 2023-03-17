//! udp socket with quic protocol as client transport layer.

mod request;
mod response;

pub use self::{request::Request, response::Response};

use core::future::Future;

use quinn::Endpoint;
use xitca_io::bytes::BytesMut;

use crate::{client::Client, config::Config, error::Error};

#[derive(Debug)]
pub(crate) struct ClientTx {
    inner: Endpoint,
}

impl ClientTx {
    pub(crate) fn is_closed(&self) -> bool {
        todo!()
    }

    pub(crate) fn send(&self, msg: BytesMut) -> Result<Response, Error> {
        todo!()
    }

    pub(crate) fn send2(&self, msg: BytesMut) -> Result<(), Error> {
        self.send(msg).map(|_| ())
    }
}

pub(crate) async fn connect(cfg: Config) -> Result<(Client, impl Future<Output = Result<(), Error>> + Send), Error> {
    Ok((todo!(), async { Ok(()) }))
}
