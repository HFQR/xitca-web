use std::future::Future;

use xitca_service::Service;

use crate::{
    builder::{marker, HttpServiceBuilder},
    error::BuildError,
};

use super::service::H2Service;

impl<St, F, Arg, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<Arg>
    for HttpServiceBuilder<marker::Http2, St, F, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: Service<Arg>,
    FA: Service,
{
    type Response = H2Service<St, F::Response, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = BuildError<FA::Error, F::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, Arg: 'f;

    fn call<'s, 'f>(&'s self, arg: Arg) -> Self::Future<'f>
    where
        's: 'f,
        Arg: 'f,
    {
        async {
            let tls_acceptor = self.tls_factory.call(()).await.map_err(BuildError::First)?;
            let service = self.factory.call(arg).await.map_err(BuildError::Second)?;
            Ok(H2Service::new(self.config, service, tls_acceptor))
        }
    }
}
