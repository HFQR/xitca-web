use std::future::Future;

use xitca_service::Service;

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
        FA: Service,
    {
        HttpServiceBuilder {
            factory: self.factory,
            tls_factory: self.tls_factory,
            config: self.config,
            _body: std::marker::PhantomData,
        }
    }
}

impl<St, F, Arg, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<Arg>
    for HttpServiceBuilder<marker::Http1, St, F, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: Service<Arg>,
    FA: Service,
{
    type Response = H1Service<St, F::Response, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = BuildError<FA::Error, F::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    fn call(&self, arg: Arg) -> Self::Future<'_> {
        async move {
            let tls_acceptor = self.tls_factory.call(()).await.map_err(BuildError::First)?;
            let service = self.factory.call(arg).await.map_err(BuildError::Second)?;
            Ok(H1Service::new(self.config, service, tls_acceptor))
        }
    }
}
