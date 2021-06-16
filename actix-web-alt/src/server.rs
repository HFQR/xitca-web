use std::{net::ToSocketAddrs, time::Duration};

use actix_http_alt::{
    config::{HttpServiceConfig, DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT},
    http::{Request, Response},
    util::ErrorLoggerFactory,
    BodyError, HttpServiceBuilder, RequestBody, ResponseBody, ResponseError,
};
use actix_server_alt::{net::Stream as ServerStream, Builder, ServerFuture};
use actix_service_alt::ServiceFactory;
use bytes::Bytes;
use futures_core::Stream;

pub struct HttpServer<F, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> {
    factory: F,
    builder: Builder,
    config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
}

impl<F, I> HttpServer<F, DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT>
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
}

impl<F, I, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServer<F, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: Fn() -> I + Send + Clone + 'static,
{
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
    ///
    /// This timeout is also used on Http/2 connection handshake phrase.
    pub fn first_request_timeout(mut self, dur: Duration) -> Self {
        self.config = self.config.first_request_timeout(dur);
        self
    }

    /// Change tls accept timeout for Http/1 and Http/2 connection.
    ///
    /// Connection can not finish tls handshake for this duration would be closed.
    pub fn tls_accept_timeout(mut self, dur: Duration) -> Self {
        self.config = self.config.tls_accept_timeout(dur);
        self
    }

    /// Change max size for request head.
    ///
    /// Request has a bigger head than it would be reject with error.
    /// Request body has a bigger continuous read would be force to yield.
    ///
    /// Default to 1mb.
    pub fn max_read_buf_size<const READ_BUF_LIMIT_2: usize>(
        self,
    ) -> HttpServer<F, HEADER_LIMIT, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT> {
        HttpServer {
            factory: self.factory,
            builder: self.builder,
            config: self.config.max_read_buf_size::<READ_BUF_LIMIT_2>(),
        }
    }

    /// Change max size for write buffer size.
    ///
    /// When write buffer hit limit it would force a drain write to Io stream until it's empty
    /// (or connection closed by error or remote peer).
    ///
    /// Default to 408kb.
    pub fn max_write_buf_size<const WRITE_BUF_LIMIT_2: usize>(
        self,
    ) -> HttpServer<F, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT_2> {
        HttpServer {
            factory: self.factory,
            builder: self.builder,
            config: self.config.max_write_buf_size::<WRITE_BUF_LIMIT_2>(),
        }
    }

    /// Change max header fields for one request.
    ///
    /// Default to 96.
    pub fn max_request_headers<const HEADER_LIMIT_2: usize>(
        self,
    ) -> HttpServer<F, HEADER_LIMIT_2, READ_BUF_LIMIT, WRITE_BUF_LIMIT> {
        HttpServer {
            factory: self.factory,
            builder: self.builder,
            config: self.config.max_request_headers::<HEADER_LIMIT_2>(),
        }
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
        let config = self.config;
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
        mut builder: openssl_crate::ssl::SslAcceptorBuilder,
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
        let config = self.config;

        const H11: &[u8] = b"\x08http/1.1";

        #[cfg(feature = "http2")]
        const H2: &[u8] = b"\x02h2";

        builder.set_alpn_select_callback(|_, protocols| {
            if protocols.windows(3).any(|window| window == H2) {
                #[cfg(feature = "http2")]
                {
                    Ok(b"h2")
                }
                #[cfg(not(feature = "http2"))]
                Err(openssl_crate::ssl::AlpnError::ALERT_FATAL)
            } else if protocols.windows(9).any(|window| window == H11) {
                Ok(b"http/1.1")
            } else {
                Err(openssl_crate::ssl::AlpnError::NOACK)
            }
        });

        #[cfg(not(feature = "http2"))]
        let protos = H11.iter().cloned().collect::<Vec<_>>();

        #[cfg(feature = "http2")]
        let protos = H11.iter().chain(H2).cloned().collect::<Vec<_>>();

        builder.set_alpn_protos(&protos)?;

        let acceptor = builder.build();

        self.builder = self
            .builder
            .bind::<_, _, _, ServerStream>("actix-web-alt", addr, move || {
                let factory = factory();
                let builder = HttpServiceBuilder::with_config(factory, config).openssl(acceptor.clone());
                ErrorLoggerFactory::new(builder)
            })?;

        Ok(self)
    }

    #[cfg(feature = "rustls")]
    pub fn bind_rustls<A: ToSocketAddrs, ResB, E>(
        mut self,
        addr: A,
        mut config: rustls_crate::ServerConfig,
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
        let service_config = self.config;

        #[cfg(feature = "http2")]
        let protos = ["h2".as_bytes().into(), "http/1.1".as_bytes().into()];

        #[cfg(not(feature = "http2"))]
        let protos = ["http/1.1".as_bytes().into()];

        config.set_protocols(protos.as_ref());

        let config = std::sync::Arc::new(config);

        self.builder = self
            .builder
            .bind::<_, _, _, ServerStream>("actix-web-alt", addr, move || {
                let factory = factory();
                let builder = HttpServiceBuilder::with_config(factory, service_config).rustls(config.clone());
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
        let config = self.config;

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
