use core::{fmt, net::SocketAddr, pin::pin};

use futures_core::Stream;
use tokio_util::sync::CancellationToken;
use xitca_io::io::{AsyncIo, PollIoAdapter};
use xitca_service::Service;

use crate::{
    bytes::Bytes,
    error::{HttpServiceError, TimeoutError},
    http::{Request, RequestExt, Response},
    service::HttpService,
    util::timer::Timeout,
};

use super::{body::RequestBody, proto::Dispatcher};

pub type H2Service<St, S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> =
    HttpService<St, S, RequestBody, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<St, S, ResB, BE, A, TlsSt, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<((St, SocketAddr), CancellationToken)>
    for H2Service<St, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    A: Service<St, Response = TlsSt>,
    St: AsyncIo,
    TlsSt: AsyncIo,
    HttpServiceError<S::Error, BE>: From<A::Error>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    BE: fmt::Debug,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;

    async fn call(
        &self,
        ((io, addr), cancellation_token): ((St, SocketAddr), CancellationToken),
    ) -> Result<Self::Response, Self::Error> {
        // tls accept timer.
        let timer = self.keep_alive();
        let mut timer = pin!(timer);

        let tls_stream = self
            .tls_acceptor
            .call(io)
            .timeout(timer.as_mut())
            .await
            .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))??;

        // update timer to first request timeout.
        self.update_first_request_deadline(timer.as_mut());

        let mut conn = ::h2::server::Builder::new()
            .enable_connect_protocol()
            .handshake(PollIoAdapter(tls_stream))
            .timeout(timer.as_mut())
            .await
            .map_err(|_| HttpServiceError::Timeout(TimeoutError::H2Handshake))??;

        let dispatcher = Dispatcher::new(
            &mut conn,
            addr,
            timer,
            self.config.keep_alive_timeout,
            &self.service,
            self.date.get(),
            cancellation_token,
        );

        dispatcher.run().await?;

        Ok(())
    }
}

#[cfg(feature = "io-uring")]
pub(crate) use io_uring::H2UringService;

#[cfg(feature = "io-uring")]
mod io_uring {
    use tokio_util::sync::CancellationToken;
    use {
        xitca_io::{
            io_uring::{AsyncBufRead, AsyncBufWrite},
            net::io_uring::TcpStream,
        },
        xitca_service::ready::ReadyService,
    };

    use crate::{
        config::HttpServiceConfig,
        date::{DateTime, DateTimeService},
        util::timer::KeepAlive,
    };

    use super::*;

    pub struct H2UringService<
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
    }

    impl<S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
        H2UringService<S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
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
            }
        }
    }

    impl<S, B, BE, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
        Service<((TcpStream, SocketAddr), CancellationToken)>
        for H2UringService<S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    where
        S: Service<Request<RequestExt<crate::h2::proto::RequestBody>>, Response = Response<B>>,
        A: Service<TcpStream>,
        A::Response: AsyncBufRead + AsyncBufWrite + 'static,
        B: Stream<Item = Result<Bytes, BE>>,
        HttpServiceError<S::Error, BE>: From<A::Error>,
        S::Error: fmt::Debug,
        BE: fmt::Debug,
    {
        type Response = ();
        type Error = HttpServiceError<S::Error, BE>;
        async fn call(
            &self,
            ((io, _), _): ((TcpStream, SocketAddr), CancellationToken),
        ) -> Result<Self::Response, Self::Error> {
            let accept_dur = self.config.tls_accept_timeout;
            let deadline = self.date.get().now() + accept_dur;
            let mut timer = pin!(KeepAlive::new(deadline));

            let io = self
                .tls_acceptor
                .call(io)
                .timeout(timer.as_mut())
                .await
                .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))??;

            crate::h2::proto::run(io, &self.service).await.unwrap();

            Ok(())
        }
    }

    impl<S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> ReadyService
        for H2UringService<S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    where
        S: ReadyService,
    {
        type Ready = S::Ready;

        #[inline]
        async fn ready(&self) -> Self::Ready {
            self.service.ready().await
        }
    }
}
