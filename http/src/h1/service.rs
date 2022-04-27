use std::future::Future;

use futures_core::Stream;
use tokio::pin;
use xitca_io::io::AsyncIo;
use xitca_service::{ready::ReadyService, Service};

use crate::{
    body::ResponseBody,
    bytes::Bytes,
    error::{HttpServiceError, TimeoutError},
    http::Response,
    request::Request,
    service::HttpService,
    util::futures::Timeout,
};

use super::{body::RequestBody, proto};

pub type H1Service<S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> =
    HttpService<S, RequestBody, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<St, S, B, BE, A, TlsSt, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<St> for H1Service<S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,

    A: Service<St, Response = TlsSt> + 'static,
    St: AsyncIo,
    TlsSt: AsyncIo,

    B: Stream<Item = Result<Bytes, BE>>,

    HttpServiceError<S::Error, BE>: From<A::Error>,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, io: St) -> Self::Future<'_> {
        async move {
            // tls accept timer.
            let timer = self.keep_alive();
            pin!(timer);

            let mut io = self
                .tls_acceptor
                .call(io)
                .timeout(timer.as_mut())
                .await
                .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))??;

            // update timer to first request timeout.
            self.update_first_request_deadline(timer.as_mut());

            proto::run(&mut io, timer.as_mut(), self.config, &self.service, self.date.get())
                .await
                .map_err(Into::into)
        }
    }
}

impl<St, S, B, BE, A, TlsSt, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    ReadyService<St> for H1Service<S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: ReadyService<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,

    A: Service<St, Response = TlsSt> + 'static,
    B: Stream<Item = Result<Bytes, BE>>,
    St: AsyncIo,
    TlsSt: AsyncIo,

    HttpServiceError<S::Error, BE>: From<A::Error>,

    B: Stream<Item = Result<Bytes, BE>>,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move { self.service.ready().await.map_err(HttpServiceError::Service) }
    }
}
