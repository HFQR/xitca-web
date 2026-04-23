use core::fmt;

use xitca_service::Service;

use crate::builder::{HttpServiceBuilder, marker};

use super::service::H2Service;

type Error = Box<dyn fmt::Debug>;

impl<St, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<marker::Http2, marker::Poll, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    #[cfg(unix)]
    pub fn unix(
        self,
    ) -> HttpServiceBuilder<
        marker::Http2,
        marker::Poll,
        xitca_io::net::UnixStream,
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
            _body: core::marker::PhantomData,
        }
    }

    /// transform Self to a http2 service builder that producing a service that able to handle [xitca_io::net::io_uring::TcpStream]
    #[cfg(feature = "io-uring")]
    pub fn io_uring(
        self,
    ) -> HttpServiceBuilder<
        marker::Http2,
        marker::Uring,
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
            _body: core::marker::PhantomData,
        }
    }
}

impl<Io, St, FA, S, E, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<Result<S, E>>
    for HttpServiceBuilder<marker::Http2, Io, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    FA: Service,
    FA::Error: fmt::Debug + 'static,
    E: fmt::Debug + 'static,
{
    type Response = H2Service<St, Io, S, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = Error;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        let service = res.map_err(|e| Box::new(e) as Error)?;
        let tls_acceptor = self.tls_factory.call(()).await.map_err(|e| Box::new(e) as Error)?;
        Ok(H2Service::new(self.config.clone(), service, tls_acceptor))
    }
}
