use std::{fmt, future::Future};

use futures_core::Stream;
use xitca_io::io::AsyncIo;
use xitca_service::ServiceFactory;

use crate::{
    body::ResponseBody,
    builder::{marker, HttpServiceBuilder},
    bytes::Bytes,
    error::HttpServiceError,
    http::Response,
    request::Request,
};

use super::{body::RequestBody, service::H2Service};

impl<
        St,
        F,
        Arg,
        ResB,
        BE,
        FA,
        TlsSt,
        const HEADER_LIMIT: usize,
        const READ_BUF_LIMIT: usize,
        const WRITE_BUF_LIMIT: usize,
    > ServiceFactory<St, Arg>
    for HttpServiceBuilder<marker::Http2, St, F, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: ServiceFactory<Request<RequestBody>, Arg, Response = Response<ResponseBody<ResB>>>,
    F::Service: 'static,
    F::Error: fmt::Debug,
    FA: ServiceFactory<St, Response = TlsSt>,
    FA::Service: 'static,
    HttpServiceError<F::Error, BE>: From<FA::Error>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
    St: AsyncIo,
    TlsSt: AsyncIo,
{
    type Response = ();
    type Error = HttpServiceError<F::Error, BE>;
    type Service = H2Service<F::Service, FA::Service, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.factory.new_service(arg);
        let tls_acceptor = self.tls_factory.new_service(());
        let config = self.config;

        async move {
            let service = service.await.map_err(HttpServiceError::Service)?;
            let tls_acceptor = tls_acceptor.await?;
            Ok(H2Service::new(config, service, tls_acceptor))
        }
    }
}
