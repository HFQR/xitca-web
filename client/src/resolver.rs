use std::net::ToSocketAddrs;

use crate::{
    connect::Connect,
    error::Error,
    service::{Service, ServiceDyn},
};

pub type ResolverService =
    Box<dyn for<'r, 'c> ServiceDyn<&'r mut Connect<'c>, Response = (), Error = Error> + Send + Sync>;

pub(crate) fn base_resolver() -> ResolverService {
    struct DefaultResolver;

    impl<'r, 'c> Service<&'r mut Connect<'c>> for DefaultResolver {
        type Response = ();
        type Error = Error;

        async fn call(&self, req: &'r mut Connect<'c>) -> Result<Self::Response, Self::Error> {
            let host = req.hostname();
            let port = req.port();

            let host = host.to_string();
            let addrs = tokio::task::spawn_blocking(move || (host, port).to_socket_addrs())
                .await
                .unwrap()?;

            req.set_addrs(addrs);

            Ok(())
        }
    }

    Box::new(DefaultResolver)
}
