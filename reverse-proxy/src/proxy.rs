use std::cell::RefCell;
use crate::peer_resolver::HttpPeerResolver;
use crate::service::{ProxyService, ProxyServiceCompat};
use crate::HttpPeer;
use std::convert::Infallible;
use std::rc::Rc;
use xitca_http::util::service::router::{PathGen, RouteGen};
use xitca_web::service::Service;

pub struct Proxy {
    peer: HttpPeer,
}

impl PathGen for Proxy {
    fn path_gen(&mut self, prefix: &str) -> String {
        let mut prefix = String::from(prefix);
        prefix.push_str("*p");
        prefix
    }
}

impl RouteGen for Proxy {
    type Route<R> = R;

    fn route_gen<R>(route: R) -> Self::Route<R> {
        route
    }
}

impl Proxy {
    pub fn new(peer: HttpPeer) -> Self {
        Self { peer }
    }
}

impl Service for Proxy {
    type Response = ProxyServiceCompat;
    type Error = Infallible;

    async fn call(&self, _: ()) -> Result<Self::Response, Self::Error> {
        Ok(ProxyServiceCompat(RefCell::new(ProxyService {
            peer_resolver: Rc::new(HttpPeerResolver::Static(Rc::new(self.peer.clone()))),
            client: Rc::new(xitca_client::ClientBuilder::new().rustls().finish()),
        })))
    }
}
