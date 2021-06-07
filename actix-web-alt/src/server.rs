use std::{net::ToSocketAddrs, time::Duration};

use actix_http_alt::{
    http::{Request, Response},
    util::ErrorLoggerFactory,
    BodyError, HttpServiceBuilder, HttpServiceConfig, RequestBody, ResponseBody, ResponseError,
};
use actix_server_alt::{net::Stream as ServerStream, Builder, ServerFuture};
use actix_service_alt::ServiceFactory;
use bytes::Bytes;
use futures_core::Stream;

pub struct HttpServer<F> {
    factory: F,
    builder: Builder,
    config: HttpServiceConfig,
}

impl<F, I> HttpServer<F>
where
    F: Fn() -> I + Send + Clone + 'static,
{
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            builder: Builder::new(),
            config: HttpServiceConfig::default(),
        }
    }

    /// Set number of threads dedicated to accepting connections.
    ///
    /// Default set to 1.
    ///
    /// # Panics:
    /// When receive 0 as number of server thread.
    pub fn server_threads(mut self, num: usize) -> Self {
        self.builder = self.builder.server_threads(num);
        self
    }

    /// Set number of workers to start.
    ///
    /// Default set to available logical cpu as workers count.
    ///
    /// # Panics:
    /// When received 0 as number of worker thread.
    pub fn worker_threads(mut self, num: usize) -> Self {
        self.builder = self.builder.worker_threads(num);
        self
    }

    /// Set max number of threads for each worker's blocking task thread pool.
    ///
    /// One thread pool is set up **per worker**; not shared across workers.
    pub fn worker_max_blocking_threads(mut self, num: usize) -> Self {
        self.builder = self.builder.worker_max_blocking_threads(num);
        self
    }

    /// Set limit of connection count for a single worker thread.
    ///
    /// When reaching limit a worker thread would enter backpressure state and stop
    /// accepting new connections until living connections reduces below the limit.
    ///
    /// A worker thread enter backpressure does not prevent other worker threads from
    /// accepting new connections as long as they have not reached their connection
    /// limits.
    ///
    /// Default set to 25_600.
    ///
    /// # Panics:
    /// When received 0 as number of connection limit.
    pub fn connection_limit(mut self, num: usize) -> Self {
        self.builder = self.builder.connection_limit(num);
        self
    }

    pub fn tcp_backlog(mut self, num: u32) -> Self {
        self.builder = self.builder.tcp_backlog(num);
        self
    }

    /// Enable Http/1 pipeline optimization.
    ///
    /// This is useful when doing benchmark and/or serving large amount of small responses.
    /// In real world it's recommended to avoid using this API.
    pub fn enable_http1_pipeline(mut self) -> Self {
        self.config = self.config.enable_http1_pipeline();
        self
    }

    /// Change keep alive duration for Http/1 connection.
    ///
    /// Connection kept idle for this duration would be closed.
    pub fn keep_alive_timeout(mut self, dur: Duration) -> Self {
        self.config = self.config.keep_alive_timeout(dur);
        self
    }

    /// Change first request timeout for Http/1 connection.
    ///
    /// Connection can not finish it's first request for this duration would be closed.
    pub fn first_request_timeout(mut self, dur: Duration) -> Self {
        self.config = self.config.first_request_timeout(dur);
        self
    }

    /// Change tls accept timeout for Http/1 and Http/2 connection.
    ///
    /// Connection can not finish tls handshake for this duration would be closed.
    ///
    /// This timeout is also used on Http/2 connection handshake phrase. Which means for a Http/2
    /// connection the total duration before it can be used to process request/response would be
    /// 2 times the tls_accept_timeout.
    pub fn tls_accept_timeout(mut self, dur: Duration) -> Self {
        self.config = self.config.tls_accept_timeout(dur);
        self
    }

    pub fn bind<A: ToSocketAddrs, ResB, E>(mut self, addr: A) -> std::io::Result<Self>
    where
        I: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<ResB>>, Config = ()> + 'static,
        I::Service: 'static,
        I::Error: ResponseError<I::Response>,
        I::InitError: From<()>,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        let factory = self.factory.clone();
        let config = self.config.clone();
        self.builder = self
            .builder
            .bind::<_, _, _, ServerStream>("actix-web-alt", addr, move || {
                let factory = factory();
                let builder = HttpServiceBuilder::with_config(factory, config);
                ErrorLoggerFactory::new(builder)
            })?;

        Ok(self)
    }

    #[cfg(feature = "openssl")]
    pub fn bind_openssl<A: ToSocketAddrs, ResB, E>(
        mut self,
        addr: A,
        builder: openssl_crate::ssl::SslAcceptorBuilder,
    ) -> std::io::Result<Self>
    where
        I: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<ResB>>, Config = ()> + 'static,
        I::Service: 'static,
        I::Error: ResponseError<I::Response>,
        I::InitError: From<()>,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        let factory = self.factory.clone();
        let config = self.config.clone();

        let acceptor = openssl_acceptor(builder)?;

        self.builder = self
            .builder
            .bind::<_, _, _, ServerStream>("actix-web-alt", addr, move || {
                let factory = factory();
                let builder = HttpServiceBuilder::with_config(factory, config);
                let builder = HttpServiceBuilder::<_, RequestBody, _, _, _>::openssl(builder, acceptor.clone());
                ErrorLoggerFactory::new(builder)
            })?;

        Ok(self)
    }

    #[cfg(unix)]
    pub fn bind_unix<P: AsRef<std::path::Path>, ResB, E>(mut self, path: P) -> std::io::Result<Self>
    where
        I: ServiceFactory<Request<RequestBody>, Response = Response<ResponseBody<ResB>>, Config = ()> + 'static,
        I::Service: 'static,
        I::Error: ResponseError<I::Response>,
        I::InitError: From<()>,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        let factory = self.factory.clone();
        let config = self.config.clone();

        self.builder = self
            .builder
            .bind_unix::<_, _, _, ServerStream>("actix-web-alt", path, move || {
                let factory = factory();
                let builder = HttpServiceBuilder::with_config(factory, config);
                ErrorLoggerFactory::new(builder)
            })?;

        Ok(self)
    }

    pub fn run(self) -> ServerFuture {
        self.builder.build()
    }
}

#[cfg(feature = "openssl")]
/// Configure `SslAcceptorBuilder` with custom server flags.
fn openssl_acceptor(
    mut builder: openssl_crate::ssl::SslAcceptorBuilder,
) -> std::io::Result<openssl_crate::ssl::SslAcceptor> {
    const H11: &[u8] = b"\x08http/1.1";
    const H2: &[u8] = b"\x02h2";

    builder.set_alpn_select_callback(|_, protocols| {
        if protocols.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else if protocols.windows(9).any(|window| window == H11) {
            Ok(b"http/1.1")
        } else {
            Err(openssl_crate::ssl::AlpnError::NOACK)
        }
    });

    let protos = H11.iter().chain(H2).cloned().collect::<Vec<_>>();

    builder.set_alpn_protos(&protos)?;

    Ok(builder.build())
}
