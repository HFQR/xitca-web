use std::{
    future::Future,
    task::{Context, Poll},
};

use actix_server_alt::net::AsyncReadWrite;
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::{ready, Stream};
use http::{Request, Response};
use tokio::{pin, select};

use crate::body::ResponseBody;
use crate::error::{BodyError, HttpServiceError, TimeoutError};
use crate::response::ResponseError;
use crate::service::HttpService;
use crate::util::keep_alive::KeepAlive;

use super::body::RequestBody;
use super::error::Error;
use super::proto::Dispatcher;

pub type H1Service<S, X, U, A, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> =
    HttpService<S, RequestBody, X, U, A, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<St, S, X, U, B, E, A, TlsSt, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<St>
    for H1Service<S, X, U, A, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    X::Error: ResponseError<S::Response>,

    U: Service<Request<RequestBody>, Response = ()> + 'static,

    A: Service<St, Response = TlsSt> + 'static,

    HttpServiceError: From<U::Error> + From<A::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    St: AsyncReadWrite,
    TlsSt: AsyncReadWrite,
{
    type Response = ();
    type Error = HttpServiceError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if let Some(upgrade) = self.flow.upgrade.as_ref() {
            ready!(upgrade.poll_ready(cx).map_err(|_| HttpServiceError::ServiceReady))?;
        }

        ready!(self
            .flow
            .expect
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady))?;

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
            let deadline = self.date.get().get().now() + accept_dur;
            let timer = KeepAlive::new(deadline);
            pin!(timer);

            select! {
                biased;
                res = self.tls_acceptor.call(io) => {
                    let mut io = res?;

                    // update timer to first request duration.
                    let request_dur = self.config.first_request_timeout;
                    let deadline = self.date.get().get().now() + request_dur;
                    timer.as_mut().update(deadline);

                    let dispatcher = Dispatcher::new(&mut io, timer.as_mut(), self.config, &*self.flow, &self.date);

                    match dispatcher.run().await {
                        Ok(_) | Err(Error::Closed) => Ok(()),
                        Err(e) => Err(e.into()),
                    }
                }
                _ = timer.as_mut() => Err(HttpServiceError::Timeout(TimeoutError::TlsAccept)),
            }
        }
    }
}
