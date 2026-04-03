use core::{net::SocketAddr, pin::pin};

use crate::body::Body;
use xitca_io::io::{AsyncBufRead, AsyncBufWrite};
use xitca_service::Service;

use crate::{
    body::RequestBody,
    bytes::Bytes,
    error::{HttpServiceError, TimeoutError},
    http::{Request, RequestExt, Response},
    service::HttpService,
    util::timer::Timeout,
};

pub type H1Service<St, S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> =
    HttpService<St, S, RequestBody, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<St, S, B, BE, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<(St, SocketAddr)> for H1Service<St, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<B>>,
    A: Service<St>,
    A::Response: AsyncBufRead + AsyncBufWrite + 'static,
    B: Body<Data = Bytes, Error = BE>,
    HttpServiceError<S::Error, BE>: From<A::Error>,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;

    async fn call(&self, (io, addr): (St, SocketAddr)) -> Result<Self::Response, Self::Error> {
        // at this stage keep-alive timer is used to tracks tls accept timeout.
        let mut timer = pin!(self.keep_alive());

        let io = self
            .tls_acceptor
            .call(io)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))??;

        super::Dispatcher::run(io, addr, timer, self.config, &self.service, self.date.get())
            .await
            .map_err(Into::into)
    }
}
