use core::fmt;

use xitca_service::Service;

use crate::builder::{marker, HttpServiceBuilder};

use super::service::H2Service;

type Error = Box<dyn fmt::Debug>;

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
