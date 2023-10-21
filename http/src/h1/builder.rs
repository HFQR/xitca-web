use xitca_service::Service;

use crate::builder::{marker, HttpServiceBuilder};

use super::service::H1Service;

impl<St, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<marker::Http1, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    #[cfg(unix)]
    /// transform Self to a http1 service builder that producing a service that able to handle [xitca_io::net::UnixStream]
    pub fn unix(
        self,
    ) -> HttpServiceBuilder<marker::Http1, xitca_io::net::UnixStream, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    where
        FA: Service,
    {
        HttpServiceBuilder {
            tls_factory: self.tls_factory,
            config: self.config,
            _body: std::marker::PhantomData,
        }
    }

    #[cfg(feature = "io-uring")]
    /// transform Self to a http1 service builder that producing a service that able to handle [xitca_io::net::io_uring::TcpStream]
    pub fn io_uring(
        self,
    ) -> HttpServiceBuilder<
        marker::Http1Uring,
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

impl<St, S, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<S>
    for HttpServiceBuilder<marker::Http1, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    FA: Service,
{
    type Response = H1Service<St, S, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = FA::Error;

    async fn call(&self, service: S) -> Result<Self::Response, Self::Error> {
        let tls_acceptor = self.tls_factory.call(()).await?;
        Ok(H1Service::new(self.config, service, tls_acceptor))
    }
}

#[cfg(feature = "io-uring")]
impl<St, S, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<S>
    for HttpServiceBuilder<marker::Http1Uring, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    FA: Service,
{
    type Response = super::service::H1UringService<S, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = FA::Error;

    async fn call(&self, service: S) -> Result<Self::Response, Self::Error> {
        let tls_acceptor = self.tls_factory.call(()).await?;
        Ok(super::service::H1UringService::new(self.config, service, tls_acceptor))
    }
}
