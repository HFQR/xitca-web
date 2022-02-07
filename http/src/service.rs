use std::{fmt, future::Future, marker::PhantomData, pin::Pin};

use futures_core::Stream;
use tokio::pin;
use xitca_io::{io::AsyncIo, net::Stream as ServerStream, net::TcpStream};
use xitca_service::Service;

use super::{
    body::{RequestBody, ResponseBody},
    bytes::Bytes,
    config::HttpServiceConfig,
    date::{DateTime, DateTimeService},
    error::{BodyError, HttpServiceError, TimeoutError},
    http::{Response, Version},
    request::Request,
    util::{futures::Timeout, keep_alive::KeepAlive},
    version::AsVersion,
};

/// General purpose http service
pub struct HttpService<
    S,
    ReqB,
    X,
    A,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> {
    pub(crate) config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    pub(crate) date: DateTimeService,
    pub(crate) expect: X,
    pub(crate) service: S,
    pub(crate) tls_acceptor: A,
    _body: PhantomData<ReqB>,
}

impl<S, ReqB, X, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpService<S, ReqB, X, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    /// Construct new Http Service.
    pub fn new(
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
        service: S,
        expect: X,
        tls_acceptor: A,
    ) -> Self {
        Self {
            config,
            date: DateTimeService::new(),
            expect,
            service,
            tls_acceptor,
            _body: PhantomData,
        }
    }

    /// Service readiness check
    pub(super) async fn _ready<ReqS, ReqX, ReqA, E>(&self) -> Result<(), HttpServiceError<E>>
    where
        S: Service<ReqS>,
        X: Service<ReqX>,
        A: Service<ReqA>,
    {
        self.expect.ready().await.map_err(|_| HttpServiceError::ServiceReady)?;

        self.tls_acceptor
            .ready()
            .await
            .map_err(|_| HttpServiceError::ServiceReady)?;

        self.service.ready().await.map_err(|_| HttpServiceError::ServiceReady)
    }

    pub(crate) fn update_first_request_deadline(&self, timer: Pin<&mut KeepAlive>) {
        let request_dur = self.config.first_request_timeout;
        let deadline = self.date.get().now() + request_dur;
        timer.update(deadline);
    }

    /// keep alive start with timer for `HttpServiceConfig.tls_accept_timeout`.
    ///
    /// It would be re-used for all following timer operation.
    ///
    /// This is an optimization for reducing heap allocation of multiple timers.
    pub(crate) fn keep_alive(&self) -> KeepAlive {
        let accept_dur = self.config.tls_accept_timeout;
        let deadline = self.date.get().now() + accept_dur;
        KeepAlive::new(deadline)
    }
}

impl<S, X, B, E, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<ServerStream> for HttpService<S, RequestBody, X, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    X: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    A: Service<TcpStream> + 'static,
    A::Response: AsyncIo + AsVersion,

    HttpServiceError<S::Error>: From<A::Error>,

    S::Error: fmt::Debug + From<X::Error>,

    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    type Response = ();
    type Error = HttpServiceError<S::Error>;
    type Ready<'f> = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        self._ready()
    }

    fn call(&self, io: ServerStream) -> Self::Future<'_> {
        async move {
            // tls accept timer.
            let timer = self.keep_alive();
            pin!(timer);

            match io {
                #[cfg(feature = "http3")]
                ServerStream::Udp(io) => super::h3::Dispatcher::new(io, &self.service)
                    .run()
                    .await
                    .map_err(From::from),
                ServerStream::Tcp(io) => {
                    #[allow(unused_mut)]
                    let mut tls_stream = self
                        .tls_acceptor
                        .call(io)
                        .timeout(timer.as_mut())
                        .await
                        .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))??;

                    let version = if self.config.peek_protocol {
                        // peek version from connection to figure out the real protocol used
                        // regardless of AsVersion's outcome.
                        todo!("peek version is not implemented yet!")
                    } else {
                        tls_stream.as_version()
                    };

                    // update timer to first request timeout.
                    self.update_first_request_deadline(timer.as_mut());

                    match version {
                        #[cfg(feature = "http1")]
                        Version::HTTP_11 | Version::HTTP_10 => super::h1::proto::run(
                            &mut tls_stream,
                            timer.as_mut(),
                            self.config,
                            &self.expect,
                            &self.service,
                            self.date.get(),
                        )
                        .await
                        .map_err(From::from),
                        #[cfg(feature = "http2")]
                        Version::HTTP_2 => {
                            let mut conn = ::h2::server::handshake(tls_stream)
                                .timeout(timer.as_mut())
                                .await
                                .map_err(|_| HttpServiceError::Timeout(TimeoutError::H2Handshake))??;

                            super::h2::Dispatcher::new(
                                &mut conn,
                                timer.as_mut(),
                                self.config.keep_alive_timeout,
                                &self.service,
                                self.date.get(),
                            )
                            .run()
                            .await
                            .map_err(Into::into)
                        }
                        version => Err(HttpServiceError::UnSupportedVersion(version)),
                    }
                }
                #[cfg(unix)]
                #[allow(unused_mut)]
                ServerStream::Unix(mut io) => {
                    #[cfg(not(feature = "http1"))]
                    {
                        drop(io);
                        Err(HttpServiceError::UnSupportedVersion(Version::HTTP_11))
                    }

                    #[cfg(feature = "http1")]
                    {
                        // update timer to first request timeout.
                        self.update_first_request_deadline(timer.as_mut());

                        super::h1::proto::run(
                            &mut io,
                            timer.as_mut(),
                            self.config,
                            &self.expect,
                            &self.service,
                            self.date.get(),
                        )
                        .await
                        .map_err(From::from)
                    }
                }
            }
        }
    }
}
