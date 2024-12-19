use crate::peer::HttpPeer;
use std::convert::Infallible;
use std::rc::Rc;
use xitca_client::Service;
use xitca_web::http::{request::Parts, Request};

pub type HttpPeerResolve = dyn Service<Request<()>, Error = Infallible, Response = Option<Rc<HttpPeer>>>;

#[derive(Clone)]
pub(crate) enum HttpPeerResolver {
    Static(Rc<HttpPeer>),
}

impl HttpPeerResolver {
    pub async fn resolve(&self, _: &Parts) -> Option<Rc<HttpPeer>> {
        match self {
            Self::Static(peer) => Some(peer.clone()),
        }
    }
}
