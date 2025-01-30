use core::{fmt, marker::PhantomData, pin::pin};

use futures_core::Stream;
use tokio_util::sync::CancellationToken;
use xitca_io::{
    io::AsyncIo,
    net::{Stream as ServerStream, TcpStream},
};
use xitca_service::{Service, ready::ReadyService};

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
    pub(crate) fn new(
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

    #[cfg(feature = "http2")]
    pub(crate) fn update_first_request_deadline(&self, timer: core::pin::Pin<&mut KeepAlive>) {
        let request_dur = self.config.request_head_timeout;
        let deadline = self.date.get().now() + request_dur;
        timer.update(deadline);
    }

    // keep alive start with timer for `HttpServiceConfig.tls_accept_timeout`.
    // It would be re-used for all following timer operation.
    // This is an optimization for reducing heap allocation of multiple timers.
    pub(crate) fn keep_alive(&self) -> KeepAlive {
        let accept_dur = self.config.tls_accept_timeout;
        let deadline = self.date.get().now() + accept_dur;
        KeepAlive::new(deadline)
    }
}

impl<S, ResB, BE, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<(ServerStream, CancellationToken)>
    for HttpService<ServerStream, S, RequestBody, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>>,
    A: Service<TcpStream>,
    A::Response: AsyncIo + AsVersion,
    HttpServiceError<S::Error, BE>: From<A::Error>,
    S::Error: fmt::Debug,
    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;

    async fn call(
        &self,
        (io, cancellation_token): (ServerStream, CancellationToken),
    ) -> Result<Self::Response, Self::Error> {
        // tls accept timer.
        let timer = self.keep_alive();
        let mut timer = pin!(timer);

        match io {
            #[cfg(feature = "http3")]
            ServerStream::Udp(io, addr) => super::h3::Dispatcher::new(io, addr, &self.service, cancellation_token)
                .run()
                .await
                .map_err(From::from),
            ServerStream::Tcp(io, _addr) => {
                let io = TcpStream::from_std(io).expect("TODO: handle io error");
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

                match version {
                    #[cfg(feature = "http1")]
                    super::http::Version::HTTP_11 | super::http::Version::HTTP_10 => super::h1::dispatcher::run(
                        &mut _tls_stream,
                        _addr,
                        timer.as_mut(),
                        self.config,
                        &self.service,
                        self.date.get(),
                        cancellation_token,
                    )
                    .await
                    .map_err(From::from),
                    #[cfg(feature = "http2")]
                    super::http::Version::HTTP_2 => {
                        // update timer to first request timeout.
                        self.update_first_request_deadline(timer.as_mut());

                        let mut conn = ::h2::server::Builder::new()
                            .enable_connect_protocol()
                            .handshake(xitca_io::io::PollIoAdapter(_tls_stream))
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
                            cancellation_token,
                        )
                        .run()
                        .await
                        .map_err(Into::into)
                    }
                    version => Err(HttpServiceError::UnSupportedVersion(version)),
                }
            }
            #[cfg(unix)]
            ServerStream::Unix(_io, _) => {
                #[cfg(not(feature = "http1"))]
                {
                    Err(HttpServiceError::UnSupportedVersion(super::http::Version::HTTP_11))
                }

                #[cfg(feature = "http1")]
                {
                    let mut io = xitca_io::net::UnixStream::from_std(_io).expect("TODO: handle io error");

                    super::h1::dispatcher::run(
                        &mut io,
                        crate::unspecified_socket_addr(),
                        timer.as_mut(),
                        self.config,
                        &self.service,
                        self.date.get(),
                        cancellation_token,
                    )
                    .await
                    .map_err(From::from)
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

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.service.ready().await
    }
}
