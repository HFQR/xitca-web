use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::{ready, Stream};
use http::{Request, Response};
use tokio::pin;
use xitca_server::net::Stream as ServerStream;
use xitca_service::Service;

use super::body::{RequestBody, ResponseBody};
use super::config::HttpServiceConfig;
use super::error::{BodyError, HttpServiceError, TimeoutError};
use super::flow::HttpFlow;
use super::protocol::AsProtocol;
use super::tls::TlsStream;
use super::util::{date::DateTimeTask, futures::Timeout, keep_alive::KeepAlive};

/// General purpose http service
pub struct HttpService<
    S,
    ReqB,
    X,
    U,
    A,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> {
    pub(crate) config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    pub(crate) date: DateTimeTask,
    pub(crate) flow: HttpFlow<S, X, U>,
    pub(crate) tls_acceptor: A,
    _body: PhantomData<ReqB>,
}

impl<S, ReqB, X, U, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpService<S, ReqB, X, U, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    /// Construct new Http Service.
    pub fn new(
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
        service: S,
        expect: X,
        upgrade: Option<U>,
        tls_acceptor: A,
    ) -> Self {
        Self {
            config,
            date: DateTimeTask::new(),
            flow: HttpFlow::new(service, expect, upgrade),
            tls_acceptor,
            _body: PhantomData,
        }
    }
}

impl<S, X, U, B, E, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<ServerStream> for HttpService<S, RequestBody, X, U, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    X: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    U: Service<Request<RequestBody>, Response = ()> + 'static,
    A: Service<ServerStream, Response = TlsStream> + 'static,

    HttpServiceError<S::Error>: From<U::Error> + From<A::Error>,

    S::Error: fmt::Debug + From<X::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    type Response = ();
    type Error = HttpServiceError<S::Error>;
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

    fn call(&self, io: ServerStream) -> Self::Future<'_> {
        async move {
            // tls accept timer.
            let accept_dur = self.config.tls_accept_timeout;
            let deadline = self.date.get().borrow().now() + accept_dur;
            let timer = KeepAlive::new(deadline);
            pin!(timer);

            match io {
                #[cfg(feature = "http3")]
                ServerStream::Udp(udp) => {
                    let dispatcher = super::h3::Dispatcher::new(udp, &self.flow);

                    dispatcher.run().await.map_err(HttpServiceError::from)
                }
                io => {
                    #[allow(unused_mut)]
                    let mut tls_stream = self
                        .tls_acceptor
                        .call(io)
                        .timeout(timer.as_mut())
                        .await
                        .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))??;

                    let protocol = tls_stream.as_protocol();

                    // update timer to first request timeout.
                    let request_dur = self.config.first_request_timeout;
                    let deadline = self.date.get().borrow().now() + request_dur;
                    timer.as_mut().update(deadline);

                    match protocol {
                        #[cfg(feature = "http1")]
                        super::protocol::Protocol::Http1Tls | super::protocol::Protocol::Http1 => {
                            super::h1::proto::run(
                                &mut tls_stream,
                                timer.as_mut(),
                                self.config,
                                &*self.flow,
                                self.date.get(),
                            )
                            .await
                            .map_err(HttpServiceError::from)
                        }
                        #[cfg(feature = "http2")]
                        super::protocol::Protocol::Http2 => {
                            let mut conn = ::h2::server::handshake(tls_stream)
                                .timeout(timer.as_mut())
                                .await
                                .map_err(|_| HttpServiceError::Timeout(TimeoutError::H2Handshake))??;

                            let dispatcher = super::h2::Dispatcher::new(
                                &mut conn,
                                timer.as_mut(),
                                self.config.keep_alive_timeout,
                                &self.flow,
                                self.date.get_shared(),
                            );

                            dispatcher.run().await.map_err(HttpServiceError::from)
                        }
                        protocol => Err(HttpServiceError::UnknownProtocol(protocol)),
                    }
                }
            }
        }
    }
}
