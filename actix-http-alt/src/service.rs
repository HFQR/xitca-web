use std::{
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use actix_server_alt::net::Stream as ServerStream;
use actix_service_alt::Service;
use bytes::Bytes;
use futures_core::{ready, Stream};
use http::{Request, Response};
use tokio::{pin, select};

use super::body::{RequestBody, ResponseBody};
use super::config::HttpServiceConfig;
use super::error::{BodyError, HttpServiceError, TimeoutError};
use super::flow::HttpFlow;
use super::protocol::{AsProtocol, Protocol};
use super::response::ResponseError;
use super::tls::TlsStream;
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
    pub fn new(config: HttpServiceConfig, service: S, expect: X, upgrade: Option<U>, tls_acceptor: A) -> Self {
        Self {
            config,
            date: DateTimeTask::new(),
            flow: HttpFlow::new(service, expect, upgrade),
            tls_acceptor,
            _body: PhantomData,
        }
    }
}

impl<S, X, U, B, E, A> Service<ServerStream> for HttpService<S, RequestBody, X, U, A>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    X::Error: ResponseError<S::Response>,

    U: Service<Request<RequestBody>, Response = ()> + 'static,

    A: Service<ServerStream, Response = TlsStream> + 'static,

    HttpServiceError: From<U::Error> + From<A::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
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

    fn call<'c>(&'c self, io: ServerStream) -> Self::Future<'c>
    where
        ServerStream: 'c,
    {
        async move {
            // tls accept timer.
            let accept_dur = self.config.tls_accept_timeout;
            let deadline = self.date.get().get().now() + accept_dur;
            let timer = KeepAlive::new(deadline);
            pin!(timer);

            match io {
                #[cfg(feature = "http3")]
                ServerStream::Udp(udp) => {
                    let dispatcher = super::h3::Dispatcher::new(udp, &self.flow);

                    dispatcher.run().await
                }
                io => select! {
                    biased;
                    res = self.tls_acceptor.call(io) => {
                        let mut tls_stream = res?;

                        let protocol = tls_stream.as_protocol();

                        match protocol {
                            #[cfg(feature = "http1")]
                            Protocol::Http1Tls | Protocol::Http1 => {
                                // update timer to first request timeout.
                                let request_dur = self.config.first_request_timeout;
                                let deadline = self.date.get().get().now() + request_dur;
                                timer.as_mut().update(deadline);

                                let dispatcher = super::h1::Dispatcher::new(&mut tls_stream, timer.as_mut(), self.config, &*self.flow, &self.date);

                                match dispatcher.run().await {
                                    Ok(_) | Err(super::h1::Error::Closed) => Ok(()),
                                    Err(e) => Err(e.into()),
                                }
                            }
                            #[cfg(feature = "http2")]
                            Protocol::Http2 => {
                                // reset timer to another accept_dur for h2 handshake timeout.
                                let deadline = self.date.get().get().now() + accept_dur;
                                timer.as_mut().update(deadline);

                                select! {
                                    biased;
                                    res = ::h2::server::handshake(tls_stream) => {
                                        let mut conn = res?;
                                        let dispatcher = super::h2::Dispatcher::new(&mut conn, &self.flow);
                                        dispatcher.run().await
                                    }
                                    _ = timer.as_mut() => Err(HttpServiceError::Timeout(TimeoutError::H2Handshake))
                                }
                            }
                            protocol => Err(HttpServiceError::UnknownProtocol(protocol))
                        }
                    }
                    _ = timer.as_mut() => Err(HttpServiceError::Timeout(TimeoutError::TlsAccept)),
                },
            }
        }
    }
}
