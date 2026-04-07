use core::{fmt, net::SocketAddr, pin::pin};

use crate::body::Body;
use xitca_io::io::{AsyncBufRead, AsyncBufWrite};
use xitca_service::Service;

use crate::{
    bytes::Bytes,
    error::{HttpServiceError, TimeoutError},
    http::{Request, RequestExt, Response},
    service::HttpService,
    util::timer::Timeout,
};

use super::{body::RequestBody, dispatcher};

pub type H2Service<St, S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> =
    HttpService<St, S, RequestBody, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<St, S, ResB, BE, A, TlsSt, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<(St, SocketAddr)> for H2Service<St, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    A: Service<St, Response = TlsSt>,
    TlsSt: AsyncBufRead + AsyncBufWrite + 'static,
    HttpServiceError<S::Error, BE>: From<A::Error>,
    ResB: Body<Data = Bytes, Error = BE>,
    BE: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;

    async fn call(&self, (io, addr): (St, SocketAddr)) -> Result<Self::Response, Self::Error> {
        // tls accept timer.
        let timer = self.keep_alive();
        let mut timer = pin!(timer);

        let io = self
            .tls_acceptor
            .call(io)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))??;

        // update timer to first request timeout.
        self.update_first_request_deadline(timer.as_mut());

        dispatcher::run(
            io,
            addr,
            crate::bytes::BytesMut::new(),
            timer,
            &self.service,
            self.date.get(),
            &self.config,
        )
        .await
        .unwrap();

        Ok(())
    }
}
