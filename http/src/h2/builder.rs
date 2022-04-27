use std::{error, future::Future};

use xitca_service::BuildService;

use crate::{
    builder::{marker, HttpServiceBuilder},
    error::BuildError,
};

use super::service::H2Service;

impl<St, F, Arg, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    BuildService<Arg> for HttpServiceBuilder<marker::Http2, St, F, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: BuildService<Arg>,
    F::Error: error::Error + 'static,

    FA: BuildService,
    FA::Error: error::Error + 'static,
{
    type Service = H2Service<F::Service, FA::Service, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = BuildError;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, arg: Arg) -> Self::Future {
        let service = self.factory.build(arg);
        let tls_acceptor = self.tls_factory.build(());
        let config = self.config;

        async move {
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;
            Ok(H2Service::new(config, service, tls_acceptor))
        }
    }
}
