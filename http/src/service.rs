use core::{fmt, marker::PhantomData, pin::pin};

use xitca_io::{
    io::{AsyncBufRead, AsyncBufWrite},
    net::{Stream, TcpStream},
};
use xitca_service::{Service, ready::ReadyService};

use super::{
    body::{Body, RequestBody},
    builder::marker,
    bytes::{Bytes, BytesMut},
    config::HttpServiceConfig,
    date::{DateTime, DateTimeService},
    error::{HttpServiceError, TimeoutError},
    http::{Request, RequestExt, Response},
    util::timer::{KeepAlive, Timeout},
    version::AsVersion,
};

// Layered sub-traits for conditional TLS acceptor bounds.
// Each sub-trait is independently cfg'd, avoiding combinatorial #[cfg] explosion
// on where clauses (which Rust doesn't support).
pub(crate) trait TlsAcceptTcp<E>:
    Service<TcpStream, Response: AsVersion + AsyncBufRead + AsyncBufWrite + 'static, Error: Into<E>>
{
}

impl<T, E> TlsAcceptTcp<E> for T where
    T: Service<TcpStream, Response: AsVersion + AsyncBufRead + AsyncBufWrite + 'static, Error: Into<E>>
{
}

#[cfg(unix)]
pub(crate) trait TlsAcceptUnix<E>:
    Service<xitca_io::net::UnixStream, Response: AsVersion + AsyncBufRead + AsyncBufWrite + 'static, Error: Into<E>>
{
}

#[cfg(unix)]
impl<T, E> TlsAcceptUnix<E> for T where
    T: Service<xitca_io::net::UnixStream, Response: AsVersion + AsyncBufRead + AsyncBufWrite + 'static, Error: Into<E>>
{
}

#[cfg(not(unix))]
pub(crate) trait TlsAcceptUnix<E> {}

#[cfg(not(unix))]
impl<T, E> TlsAcceptUnix<E> for T {}

#[cfg(feature = "io-uring")]
pub(crate) trait TlsAcceptUring<E>:
    Service<
        xitca_io::net::io_uring::TcpStream,
        Response: AsVersion + AsyncBufRead + AsyncBufWrite + 'static,
        Error: Into<E>,
    >
{
}

#[cfg(feature = "io-uring")]
impl<T, E> TlsAcceptUring<E> for T where
    T: Service<
            xitca_io::net::io_uring::TcpStream,
            Response: AsVersion + AsyncBufRead + AsyncBufWrite + 'static,
            Error: Into<E>,
        >
{
}

#[cfg(not(feature = "io-uring"))]
pub(crate) trait TlsAcceptUring<E> {}

#[cfg(not(feature = "io-uring"))]
impl<T, E> TlsAcceptUring<E> for T {}

#[cfg(feature = "io-uring")]
pub(crate) trait TlsAcceptUnixUring<E>:
    Service<
        xitca_io::net::io_uring::UnixStream,
        Response: AsVersion + AsyncBufRead + AsyncBufWrite + 'static,
        Error: Into<E>,
    >
{
}

#[cfg(feature = "io-uring")]
impl<T, E> TlsAcceptUnixUring<E> for T where
    T: Service<
            xitca_io::net::io_uring::UnixStream,
            Response: AsVersion + AsyncBufRead + AsyncBufWrite + 'static,
            Error: Into<E>,
        >
{
}

#[cfg(not(feature = "io-uring"))]
pub(crate) trait TlsAcceptUnixUring<E> {}

#[cfg(not(feature = "io-uring"))]
impl<T, E> TlsAcceptUnixUring<E> for T {}

pub(crate) trait TlsAccept<E>:
    TlsAcceptTcp<E> + TlsAcceptUnix<E> + TlsAcceptUring<E> + TlsAcceptUnixUring<E>
{
}
impl<T, E> TlsAccept<E> for T where T: TlsAcceptTcp<E> + TlsAcceptUnix<E> + TlsAcceptUring<E> + TlsAcceptUnixUring<E> {}

pub struct HttpService<
    V,
    Io,
    St,
    S,
    A,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> {
    pub(crate) config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    pub(crate) date: DateTimeService,
    pub(crate) service: S,
    pub(crate) tls_acceptor: A,
    _marker: PhantomData<(V, Io, St)>,
}

impl<V, Io, St, S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpService<V, Io, St, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
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
            _marker: PhantomData,
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

impl<S, Io, B, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpService<marker::Http, Io, Stream, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<B>>,
    S::Error: fmt::Debug,
    B: Body<Data = Bytes>,
    B::Error: fmt::Debug,
{
    async fn dispatch(
        &self,
        _tls_stream: impl AsVersion + AsyncBufRead + AsyncBufWrite + 'static,
        _addr: core::net::SocketAddr,
        mut _timer: core::pin::Pin<&mut KeepAlive>,
    ) -> Result<(), HttpServiceError<S::Error, B::Error>> {
        #[allow(unused_mut)]
        let mut version = _tls_stream.as_version();
        #[allow(unused_mut)]
        let mut _read_buf = BytesMut::new();

        #[cfg(feature = "http2")]
        if self.config.peek_protocol {
            let (ver, buf) = super::h2::dispatcher::peek_version(&_tls_stream, BytesMut::new())
                .timeout(_timer.as_mut())
                .await
                // TODO: more precise error handling
                .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))?
                .map_err(super::h2::Error::Io)?;
            version = ver;
            _read_buf = buf;
        };

        match version {
            #[cfg(feature = "http1")]
            super::http::Version::HTTP_11 | super::http::Version::HTTP_10 => super::h1::Dispatcher::run(
                _tls_stream,
                _addr,
                _read_buf,
                _timer.as_mut(),
                self.config,
                &self.service,
                self.date.get(),
            )
            .await
            .map_err(From::from),
            #[cfg(feature = "http2")]
            super::http::Version::HTTP_2 => {
                // update timer to first request timeout.
                self.update_first_request_deadline(_timer.as_mut());

                super::h2::dispatcher::run(
                    _tls_stream,
                    _addr,
                    _read_buf,
                    _timer.as_mut(),
                    &self.service,
                    self.date.get(),
                    &self.config,
                )
                .await
                .map_err(super::h2::Error::Io)?;

                Ok(())
            }
            version => Err(HttpServiceError::UnSupportedVersion(version)),
        }
    }
}

#[cfg(feature = "io-uring")]
impl<S, B, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<Stream>
    for HttpService<marker::Http, marker::Uring, Stream, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<B>>,
    S::Error: fmt::Debug,
    A: TlsAccept<HttpServiceError<S::Error, B::Error>>,
    B: Body<Data = Bytes>,
    B::Error: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, B::Error>;

    async fn call(&self, io: Stream) -> Result<Self::Response, Self::Error> {
        // tls accept timer.
        let timer = self.keep_alive();
        let mut timer = pin!(timer);

        match io {
            #[cfg(feature = "http3")]
            Stream::Udp(io, addr) => super::h3::Dispatcher::new(io, addr, &self.service)
                .run()
                .await
                .map_err(From::from),
            Stream::Tcp(io, _addr) => {
                let io = xitca_io::net::io_uring::TcpStream::from_std(io);
                let _tls_stream = self
                    .tls_acceptor
                    .call(io)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))?
                    .map_err(Into::into)?;

                self.dispatch(_tls_stream, _addr, timer.as_mut()).await
            }
            #[cfg(unix)]
            Stream::Unix(_io, _) => {
                let io = xitca_io::net::io_uring::UnixStream::from_std(_io);
                let _tls_stream = self
                    .tls_acceptor
                    .call(io)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))?
                    .map_err(Into::into)?;

                self.dispatch(_tls_stream, crate::unspecified_socket_addr(), timer.as_mut())
                    .await
            }
        }
    }
}

impl<S, B, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> Service<Stream>
    for HttpService<marker::Http, marker::Poll, Stream, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<B>>,
    S::Error: fmt::Debug,
    A: TlsAccept<HttpServiceError<S::Error, B::Error>>,
    B: Body<Data = Bytes>,
    B::Error: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, B::Error>;

    async fn call(&self, io: Stream) -> Result<Self::Response, Self::Error> {
        // tls accept timer.
        let timer = self.keep_alive();
        let mut timer = pin!(timer);

        match io {
            #[cfg(feature = "http3")]
            Stream::Udp(io, addr) => super::h3::Dispatcher::new(io, addr, &self.service)
                .run()
                .await
                .map_err(From::from),
            Stream::Tcp(io, _addr) => {
                let io = TcpStream::from_std(io).expect("TODO: handle io error");
                let _tls_stream = self
                    .tls_acceptor
                    .call(io)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))?
                    .map_err(Into::into)?;

                self.dispatch(_tls_stream, _addr, timer.as_mut()).await
            }
            #[cfg(unix)]
            Stream::Unix(_io, _) => {
                let io = xitca_io::net::UnixStream::from_std(_io).expect("TODO: handle io error");
                let _tls_stream = self
                    .tls_acceptor
                    .call(io)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))?
                    .map_err(Into::into)?;

                self.dispatch(_tls_stream, crate::unspecified_socket_addr(), timer.as_mut())
                    .await
            }
        }
    }
}

impl<V, Io, St, S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> ReadyService
    for HttpService<V, Io, St, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.service.ready().await
    }
}
