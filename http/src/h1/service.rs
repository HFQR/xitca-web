use core::{net::SocketAddr, pin::pin};

use std::sync::Arc;

use crate::body::Body;
use xitca_io::io::{AsyncBufRead, AsyncBufWrite};
use xitca_service::{Service, shutdown::ShutdownToken};

use crate::{
    body::RequestBody,
    builder::marker,
    bytes::Bytes,
    error::{HttpServiceError, TimeoutError},
    http::{Request, RequestExt, Response},
    service::HttpService,
    util::timer::Timeout,
};

pub type H1Service<St, Io, S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> =
    HttpService<marker::Http1, Io, St, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<St, Io, S, B, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<((St, SocketAddr), Arc<ShutdownToken>)>
    for H1Service<St, Io, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<B>>,
    A: Service<St>,
    A::Response: AsyncBufRead + AsyncBufWrite + 'static,
    B: Body<Data = Bytes>,
    HttpServiceError<S::Error, B::Error>: From<A::Error>,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, B::Error>;

    async fn call(
        &self,
        ((io, addr), st): ((St, SocketAddr), Arc<ShutdownToken>),
    ) -> Result<Self::Response, Self::Error> {
        // at this stage keep-alive timer is used to tracks tls accept timeout.
        let mut timer = pin!(self.keep_alive());

        let io = self
            .tls_acceptor
            .call(io)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))??;

        super::Dispatcher::run(
            io,
            addr,
            crate::bytes::BytesMut::new(),
            timer,
            self.config,
            &self.service,
            self.date.get(),
            &st,
        )
        .await
        .map_err(Into::into)
    }
}
