use std::future::Future;

use futures_core::Stream;
use xitca_io::io::AsyncIo;
use xitca_service::ServiceFactory;

use crate::{
    body::ResponseBody,
    builder::HttpServiceBuilder,
    bytes::Bytes,
    error::{BodyError, HttpServiceError},
    http::{Request, Response},
};

use super::{body::RequestBody, service::H1Service};

/// Http/1 Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub type H1ServiceBuilder<
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
        ResB,
        E,
        FE,
        FU,
        FA,
        TlsSt,
        const HEADER_LIMIT: usize,
        const READ_BUF_LIMIT: usize,
        const WRITE_BUF_LIMIT: usize,
    > ServiceFactory<St> for H1ServiceBuilder<F, FE, FU, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<ResB>>>,
    F::Service: 'static,
    F::InitError: From<FE::InitError> + From<FU::InitError> + From<FA::InitError>,

    // TODO: use a meaningful config.
    FE: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>, Config = ()>,
    FE::Service: 'static,

    // TODO: use a meaningful config.
    FU: ServiceFactory<Request<RequestBody>, Response = (), Config = ()>,
    FU::Service: 'static,

    FA: ServiceFactory<St, Response = TlsSt, Config = ()>,
    FA::Service: 'static,

    HttpServiceError<F::Error>: From<FU::Error> + From<FA::Error>,
    F::Error: From<FE::Error>,

    ResB: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncIo,
    TlsSt: AsyncIo,
{
    type Response = ();
    type Error = HttpServiceError<F::Error>;
    type Config = F::Config;
    type Service =
        H1Service<F::Service, FE::Service, FU::Service, FA::Service, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let expect = self.expect.new_service(());
        let upgrade = self.upgrade.as_ref().map(|upgrade| upgrade.new_service(()));
        let service = self.factory.new_service(cfg);
        let tls_acceptor = self.tls_factory.new_service(());
        let config = self.config;

        async move {
            let expect = expect.await?;
            let upgrade = match upgrade {
                Some(upgrade) => Some(upgrade.await?),
                None => None,
            };
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;

            Ok(H1Service::new(config, service, expect, upgrade, tls_acceptor))
        }
    }
}
