use xitca_service::Service;

use crate::builder::{marker, HttpServiceBuilder};

use super::service::H2Service;

impl<St, S, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<S>
    for HttpServiceBuilder<marker::Http2, St, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    FA: Service,
{
    type Response = H2Service<St, S, FA::Response, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Error = FA::Error;

    async fn call(&self, service: S) -> Result<Self::Response, Self::Error> {
        let tls_acceptor = self.tls_factory.call(()).await?;
        Ok(H2Service::new(self.config, service, tls_acceptor))
    }
}
