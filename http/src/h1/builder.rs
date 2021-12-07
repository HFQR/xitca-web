use std::future::Future;

use futures_core::Stream;
use xitca_io::io::AsyncIo;
use xitca_service::ServiceFactory;

use crate::{
    body::ResponseBody,
    builder::{marker, HttpServiceBuilder},
    bytes::Bytes,
    error::{BodyError, HttpServiceError},
    http::{Request, Response},
};

use super::{body::RequestBody, service::H1Service};

#[cfg(unix)]
impl<St, F, FE, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<marker::Http1, St, F, FE, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    /// Transform Self to a Http1 service builder that able to take in [xitca_io::net::UnixStream] IO type.
    pub fn unix(
        self,
    ) -> HttpServiceBuilder<
        marker::Http1,
        xitca_io::net::UnixStream,
        F,
        FE,
        FA,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    >
    where
        FA: ServiceFactory<xitca_io::net::UnixStream>,
    {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            tls_factory: self.tls_factory,
            config: self.config,
            _body: std::marker::PhantomData,
        }
    }
}

impl<
        St,
        F,
        ResB,
        E,
        FE,
        FA,
        TlsSt,
        const HEADER_LIMIT: usize,
        const READ_BUF_LIMIT: usize,
        const WRITE_BUF_LIMIT: usize,
    > ServiceFactory<St>
    for HttpServiceBuilder<marker::Http1, St, F, FE, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<ResB>>>,
    F::Service: 'static,
    F::InitError: From<FE::InitError> + From<FA::InitError>,

    // TODO: use a meaningful config.
    FE: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>, Config = ()>,
    FE::Service: 'static,

    FA: ServiceFactory<St, Response = TlsSt, Config = ()>,
    FA::Service: 'static,

    HttpServiceError<F::Error>: From<FA::Error>,
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
    type Service = H1Service<F::Service, FE::Service, FA::Service, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let expect = self.expect.new_service(());
        let service = self.factory.new_service(cfg);
        let tls_acceptor = self.tls_factory.new_service(());
        let config = self.config;

        async move {
            let expect = expect.await?;
            let service = service.await?;
            let tls_acceptor = tls_acceptor.await?;
            Ok(H1Service::new(config, service, expect, tls_acceptor))
        }
    }
}
