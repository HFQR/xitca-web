use core::fmt;

use xitca_service::Service;

use crate::builder::{HttpServiceBuilder, marker};

use super::service::H2Service;

type Error = Box<dyn fmt::Debug>;

#[cfg(feature = "io-uring")]
impl<St, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<marker::Http2, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    /// transform Self to a http2 service builder that producing a service that able to handle [xitca_io::net::io_uring::TcpStream]
    pub fn io_uring(
        self,
    ) -> HttpServiceBuilder<
        marker::Http2Uring,
        xitca_io::net::io_uring::TcpStream,
        FA,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    >
    where
        FA: Service,
    {
        HttpServiceBuilder {
            tls_factory: self.tls_factory,
            config: self.config,
            _body: std::marker::PhantomData,
        }
    }
}

impl<St, FA, S, E, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<Result<S, E>> for HttpServiceBuilder<marker::Http2, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    FA: Service,
    FA::Error: fmt::Debug + 'static,
    E: fmt::Debug + 'static,
{
    type Response = H2Service<St, S, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = Error;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        let service = res.map_err(|e| Box::new(e) as Error)?;
        let tls_acceptor = self.tls_factory.call(()).await.map_err(|e| Box::new(e) as Error)?;
        Ok(H2Service::new(self.config, service, tls_acceptor))
    }
}

#[cfg(feature = "io-uring")]
impl<St, FA, S, E, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<Result<S, E>>
    for HttpServiceBuilder<marker::Http2Uring, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    FA: Service,
    FA::Error: fmt::Debug + 'static,
    E: fmt::Debug + 'static,
{
    type Response = super::service::H2UringService<S, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = Error;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        let service = res.map_err(|e| Box::new(e) as Error)?;
        let tls_acceptor = self.tls_factory.call(()).await.map_err(|e| Box::new(e) as Error)?;
        Ok(super::service::H2UringService::new(self.config, service, tls_acceptor))
    }
}
