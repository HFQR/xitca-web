use std::{
    future::Future,
    task::{Context, Poll},
};

use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::{ready, Stream};
use http::{Request, Response};
use tokio::{pin, select};

use crate::body::ResponseBody;
use crate::config::HttpServiceConfig;
use crate::error::{BodyError, HttpServiceError};
use crate::flow::HttpFlow;
use crate::response::ResponseError;
use crate::stream::AsyncStream;
use crate::util::date::DateTimeTask;

use super::body::RequestBody;
use super::error::Error;
use super::proto::{Dispatcher, KeepAlive};

pub struct H1Service<S, X, U, A> {
    config: HttpServiceConfig,
    date: DateTimeTask,
    flow: HttpFlow<S, X, U>,
    tls_acceptor: A,
}

impl<S, X, U, A> H1Service<S, X, U, A> {
    /// Construct new Http1Service.
    pub fn new(config: HttpServiceConfig, service: S, expect: X, upgrade: U, tls_acceptor: A) -> Self {
        Self {
            config,
            date: DateTimeTask::new(),
            flow: HttpFlow::new(service, expect, upgrade),
            tls_acceptor,
        }
    }
}

impl<St, S, X, U, B, E, A, TlsSt> Service<St> for H1Service<S, X, U, A>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    X::Error: ResponseError<S::Response>,

    U: 'static,

    A: Service<St, Response = TlsSt> + 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    HttpServiceError: From<A::Error>,

    St: AsyncStream,
    TlsSt: AsyncStream,
{
    type Response = ();
    type Error = HttpServiceError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // ready!(self
        //     .flow
        //     .upgrade
        //     .poll_ready(cx)
        //     .map_err(|_| HttpServiceError::ServiceReady))?;
        //
        ready!(self
            .flow
            .expect
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady))?;

        self.flow
            .service
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady)
    }

    fn call<'c>(&'c self, io: St) -> Self::Future<'c>
    where
        St: 'c,
    {
        async move {
            // tls accept timer.
            let accept_dur = self.config.tls_accept_dur;
            let deadline = self.date.get().get().now() + accept_dur;
            let timer = KeepAlive::new(deadline);
            pin!(timer);

            select! {
                res = self.tls_acceptor.call(io) => {
                    let mut io = res?;

                    // update timer to first request duration.
                    let request_dur = self.config.first_request_dur;
                    let deadline = self.date.get().get().now() + request_dur;
                    timer.as_mut().update(deadline);

                    let mut dispatcher = Dispatcher::new(&mut io, timer.as_mut(), self.config, &self.flow, &self.date);

                    match dispatcher.run().await {
                        Ok(_) | Err(Error::Closed) => Ok(()),
                        Err(e) => Err(e.into()),
                    }
                }
                _ = timer.as_mut() => Err(HttpServiceError::TlsAcceptTimeout),
            }
        }
    }
}
