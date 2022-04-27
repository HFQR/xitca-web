use std::{error, future::Future};

use xitca_service::BuildService;

use crate::{
    builder::{marker, HttpServiceBuilder},
    error::BuildError,
};

use super::service::H1Service;

#[cfg(unix)]
impl<St, F, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<marker::Http1, St, F, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    /// Transform Self to a Http1 service builder that able to take in [xitca_io::net::UnixStream] IO type.
    pub fn unix(
        self,
    ) -> HttpServiceBuilder<
        marker::Http1,
        xitca_io::net::UnixStream,
        F,
        FA,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    >
    where
        FA: BuildService<xitca_io::net::UnixStream>,
    {
        HttpServiceBuilder {
            factory: self.factory,
            tls_factory: self.tls_factory,
            config: self.config,
            _body: std::marker::PhantomData,
        }
    }
}

impl<St, F, Arg, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    BuildService<Arg> for HttpServiceBuilder<marker::Http1, St, F, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: BuildService<Arg>,
    F::Error: error::Error + 'static,

    FA: BuildService,
    FA::Error: error::Error + 'static,
{
    type Service = H1Service<F::Service, FA::Service, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = BuildError;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let service = self.factory.build(arg);
        let tls_acceptor = self.tls_factory.build(());
        let config = self.config;

        async move {
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;
            Ok(H1Service::new(config, service, tls_acceptor))
        }
    }
}
