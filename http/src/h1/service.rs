use std::future::Future;

use futures_core::Stream;
use http::{Request, Response};
use tokio::pin;
use xitca_io::io::AsyncIo;
use xitca_service::Service;

use crate::{
    body::ResponseBody,
    bytes::Bytes,
    error::{BodyError, HttpServiceError, TimeoutError},
    service::HttpService,
    util::futures::Timeout,
};

use super::{body::RequestBody, proto};

pub type H1Service<S, X, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> =
    HttpService<S, RequestBody, X, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<
        St,
        S,
        X,
        B,
        E,
        A,
        TlsSt,
        const HEADER_LIMIT: usize,
        const READ_BUF_LIMIT: usize,
        const WRITE_BUF_LIMIT: usize,
    > Service<St> for H1Service<S, X, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    X: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    A: Service<St, Response = TlsSt> + 'static,

    S::Error: From<X::Error>,
    HttpServiceError<S::Error>: From<A::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncIo,
    TlsSt: AsyncIo,
{
    type Response = ();
    type Error = HttpServiceError<S::Error>;
    type Ready<'f> = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        self._ready()
    }

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

            proto::run(
                &mut io,
                timer.as_mut(),
                self.config,
                &self.expect,
                &self.service,
                self.date.get(),
            )
            .await
            .map_err(Into::into)
        }
    }
}
