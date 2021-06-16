use std::{
    future::Future,
    task::{Context, Poll},
};

use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::{ready, Stream};
use http::{Request, Response};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    pin, select,
};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError, TimeoutError};
use crate::response::ResponseError;
use crate::service::HttpService;
use crate::util::keep_alive::KeepAlive;

use super::body::RequestBody;
use super::proto::Dispatcher;

pub type H2Service<S, A, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> =
    HttpService<S, RequestBody, (), (), A, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<St, S, B, E, A, TlsSt, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<St>
    for H2Service<S, A, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    A: Service<St, Response = TlsSt> + 'static,

    S::Error: ResponseError<S::Response>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncRead + AsyncWrite + Unpin,
    TlsSt: AsyncRead + AsyncWrite + Unpin,

    HttpServiceError: From<A::Error>,
{
    type Response = ();
    type Error = HttpServiceError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self
            .tls_acceptor
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady))?;

        self.flow
            .service
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady)
    }

    fn call(&self, io: St) -> Self::Future<'_> {
        async move {
            // tls accept timer.
            let accept_dur = self.config.tls_accept_timeout;
            let deadline = self.date.get().borrow().now() + accept_dur;
            let timer = KeepAlive::new(deadline);
            pin!(timer);

            select! {
                biased;
                res = self.tls_acceptor.call(io) => {
                    let tls_stream = res?;

                    // update timer to first request timeout.
                    let request_dur = self.config.first_request_timeout;
                    let deadline = self.date.get().borrow().now() + request_dur;
                    timer.as_mut().update(deadline);

                    select! {
                        biased;
                        res = ::h2::server::handshake(tls_stream) => {
                            let mut conn = res?;

                            let dispatcher = Dispatcher::new(&mut conn, timer.as_mut(), self.config.keep_alive_timeout, &self.flow, self.date.get_shared());
                            dispatcher.run().await?;

                            Ok(())
                        }
                        _ = timer.as_mut() => Err(HttpServiceError::Timeout(TimeoutError::H2Handshake))
                    }
                }
                _ = timer.as_mut() => Err(HttpServiceError::Timeout(TimeoutError::TlsAccept)),
            }
        }
    }
}
