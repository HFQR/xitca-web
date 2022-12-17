use std::{fmt, future::Future, marker::PhantomData, pin::Pin};

use futures_core::Stream;
use xitca_io::{
    io::{AsyncIo, AsyncRead, AsyncWrite},
    net::Stream as ServerStream,
    net::TcpStream,
};
use xitca_service::{ready::ReadyService, Service};
use xitca_unsafe_collection::pin;

use super::{
    body::RequestBody,
    bytes::Bytes,
    config::HttpServiceConfig,
    date::{DateTime, DateTimeService},
    error::{HttpServiceError, TimeoutError},
    http::{Request, RequestExt, Response},
    util::timer::{KeepAlive, Timeout},
    version::AsVersion,
};

/// General purpose http service
pub struct HttpService<
    St,
    S,
    ReqB,
    A,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> {
    pub(crate) config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    pub(crate) date: DateTimeService,
    pub(crate) service: S,
    pub(crate) tls_acceptor: A,
    _body: PhantomData<(St, ReqB)>,
}

impl<St, S, ReqB, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpService<St, S, ReqB, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    /// Construct new Http Service.
    pub fn new(
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
        service: S,
        tls_acceptor: A,
    ) -> Self {
        Self {
            config,
            date: DateTimeService::new(),
            service,
            tls_acceptor,
            _body: PhantomData,
        }
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

impl<S, ResB, BE, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<ServerStream>
    for HttpService<ServerStream, S, RequestBody, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>>,
    A: Service<TcpStream>,
    A::Response: AsyncIo + AsVersion + AsyncRead + AsyncWrite + Unpin,
    HttpServiceError<S::Error, BE>: From<A::Error>,
    S::Error: fmt::Debug,
    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f;

    fn call<'s>(&'s self, io: ServerStream) -> Self::Future<'s>
    where
        ServerStream: 's,
    {
        async {
            // tls accept timer.
            let timer = self.keep_alive();
            pin!(timer);

            match io {
                #[cfg(feature = "http3")]
                ServerStream::Udp(io, addr) => super::h3::Dispatcher::new(io, addr, &self.service)
                    .run()
                    .await
                    .map_err(From::from),
                ServerStream::Tcp(io, _addr) => {
                    let mut _tls_stream = self
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
                        _tls_stream.as_version()
                    };

                    // update timer to first request timeout.
                    self.update_first_request_deadline(timer.as_mut());

                    match version {
                        #[cfg(feature = "http1")]
                        super::http::Version::HTTP_11 | super::http::Version::HTTP_10 => super::h1::proto::run(
                            &mut _tls_stream,
                            _addr,
                            timer.as_mut(),
                            self.config,
                            &self.service,
                            self.date.get(),
                        )
                        .await
                        .map_err(From::from),
                        #[cfg(feature = "http2")]
                        super::http::Version::HTTP_2 => {
                            let mut conn = ::h2::server::Builder::new()
                                .enable_connect_protocol()
                                .handshake(_tls_stream)
                                .timeout(timer.as_mut())
                                .await
                                .map_err(|_| HttpServiceError::Timeout(TimeoutError::H2Handshake))??;

                            super::h2::Dispatcher::new(
                                &mut conn,
                                _addr,
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
                ServerStream::Unix(mut _io, _) => {
                    #[cfg(not(feature = "http1"))]
                    {
                        Err(HttpServiceError::UnSupportedVersion(super::http::Version::HTTP_11))
                    }

                    #[cfg(feature = "http1")]
                    {
                        // update timer to first request timeout.
                        self.update_first_request_deadline(timer.as_mut());

                        super::h1::proto::run(
                            &mut _io,
                            crate::unspecified_socket_addr(),
                            timer.as_mut(),
                            self.config,
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

impl<St, S, ReqB, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> ReadyService
    for HttpService<St, S, ReqB, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}
