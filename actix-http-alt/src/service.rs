use std::{
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use actix_server_alt::net::AsyncReadWrite;
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::{ready, Stream};
use http::{Request, Response};
use tokio::{pin, select};

use super::body::{RequestBody, ResponseBody};
use super::config::HttpServiceConfig;
use super::error::{BodyError, HttpServiceError};
use super::flow::HttpFlow;
use super::h1::{Dispatcher, Error};
use super::response::ResponseError;
use super::util::{date::DateTimeTask, keep_alive::KeepAlive};

pub struct HttpService<S, ReqB, X, U, A> {
    pub(crate) config: HttpServiceConfig,
    pub(crate) date: DateTimeTask,
    pub(crate) flow: HttpFlow<S, X, U>,
    pub(crate) tls_acceptor: A,
    _body: PhantomData<ReqB>,
}

impl<S, ReqB, X, U, A> HttpService<S, ReqB, X, U, A> {
    /// Construct new Http Service.
    pub fn new(config: HttpServiceConfig, service: S, expect: X, upgrade: U, tls_acceptor: A) -> Self {
        Self {
            config,
            date: DateTimeTask::new(),
            flow: HttpFlow::new(service, expect, upgrade),
            tls_acceptor,
            _body: PhantomData,
        }
    }
}

impl<St, S, X, U, B, E, A, TlsSt> Service<St> for HttpService<S, RequestBody, X, U, A>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    X::Error: ResponseError<S::Response>,

    U: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    U::Error: ResponseError<S::Response>,

    A: Service<St, Response = TlsSt> + 'static,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,

    HttpServiceError: From<A::Error>,

    St: AsyncReadWrite,
    TlsSt: AsyncReadWrite,
{
    type Response = ();
    type Error = HttpServiceError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self
            .flow
            .upgrade
            .poll_ready(cx)
            .map_err(|_| HttpServiceError::ServiceReady))?;

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
                biased;
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
