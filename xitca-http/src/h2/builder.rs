use std::{fmt, future::Future};

use bytes::Bytes;
use futures_core::Stream;
use http::{Request, Response};
use tokio::io::{AsyncRead, AsyncWrite};
use xitca_service::ServiceFactory;

use crate::body::ResponseBody;
use crate::builder::HttpServiceBuilder;
use crate::error::{BodyError, HttpServiceError};

use super::body::RequestBody;
use super::service::H2Service;

/// Http/1 Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub type H2ServiceBuilder<
    F,
    FE,
    FU,
    FA,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> = HttpServiceBuilder<F, RequestBody, FE, FU, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<
        St,
        F,
        B,
        E,
        FE,
        FU,
        FA,
        TlsSt,
        const HEADER_LIMIT: usize,
        const READ_BUF_LIMIT: usize,
        const WRITE_BUF_LIMIT: usize,
    > ServiceFactory<St> for H2ServiceBuilder<F, FE, FU, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<B>>>,
    F::Service: 'static,
    F::Error: fmt::Debug,
    F::InitError: From<FA::InitError>,

    FA: ServiceFactory<St, Response = TlsSt, Config = ()>,
    FA::Service: 'static,
    HttpServiceError<F::Error>: From<FA::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncRead + AsyncWrite + Unpin,
    TlsSt: AsyncRead + AsyncWrite + Unpin,
{
    type Response = ();
    type Error = HttpServiceError<F::Error>;
    type Config = F::Config;
    type Service = H2Service<F::Service, FA::Service, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let tls_acceptor = self.tls_factory.new_service(());
        let config = self.config;

        async move {
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;

            Ok(H2Service::new(config, service, (), None, tls_acceptor))
        }
    }
}
