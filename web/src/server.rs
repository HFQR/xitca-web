use std::{fmt, future::Future, time::Duration};

use futures_core::stream::Stream;
use xitca_http::{
    body::RequestBody,
    config::{HttpServiceConfig, DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT},
    HttpServiceBuilder,
};
use xitca_server::{Builder, ServerFuture};

use crate::{
    dev::{
        bytes::Bytes,
        service::{ready::ReadyService, Service},
    },
    http::{Request, RequestExt, Response},
};

pub struct HttpServer<
    F,
    const HEADER_LIMIT: usize = DEFAULT_HEADER_LIMIT,
    const READ_BUF_LIMIT: usize = DEFAULT_READ_BUF_LIMIT,
    const WRITE_BUF_LIMIT: usize = DEFAULT_WRITE_BUF_LIMIT,
> {
    factory: F,
    builder: Builder,
    config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
}

impl<F, I> HttpServer<F>
where
    F: Fn() -> I + Send + Sync + Clone + 'static,
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
    F: Fn() -> I + Send + Sync + Clone + 'static,
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

    /// Disable signal listening.
    ///
    /// `tokio::signal` is used for listening and it only functions in tokio runtime 1.x.
    /// Disabling it would enable server runs in other async runtimes.
    pub fn disable_signal(mut self) -> Self {
        self.builder = self.builder.disable_signal();
        self
    }

    pub fn backlog(mut self, num: u32) -> Self {
        self.builder = self.builder.backlog(num);
        self
    }

    /// Disable vectored write even when IO is able to perform it.
    ///
    /// This is beneficial when dealing with small size of response body.
    pub fn disable_vectored_write(mut self) -> Self {
        self.config = self.config.disable_vectored_write();
        self
    }

    /// Change keep alive duration for Http/1 connection.
    ///
    /// Connection kept idle for this duration would be closed.
    pub fn keep_alive_timeout(mut self, dur: Duration) -> Self {
        self.config = self.config.keep_alive_timeout(dur);
        self
    }

    /// Change request timeout for Http/1 connection.
    ///
    /// Connection can not finish it's request for this duration would be closed.
    ///
    /// This timeout is also used in Http/2 connection handshake phrase.
    pub fn request_head_timeout(mut self, dur: Duration) -> Self {
        self.config = self.config.request_head_timeout(dur);
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
        self.mutate_const_generic::<HEADER_LIMIT, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT>()
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
        self.mutate_const_generic::<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT_2>()
    }

    /// Change max header fields for one request.
    ///
    /// Default to 64.
    pub fn max_request_headers<const HEADER_LIMIT_2: usize>(
        self,
    ) -> HttpServer<F, HEADER_LIMIT_2, READ_BUF_LIMIT, WRITE_BUF_LIMIT> {
        self.mutate_const_generic::<HEADER_LIMIT_2, READ_BUF_LIMIT, WRITE_BUF_LIMIT>()
    }

    #[doc(hidden)]
    pub fn on_worker_start<FS, Fut>(mut self, on_start: FS) -> Self
    where
        FS: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future + Send + 'static,
    {
        self.builder = self.builder.on_worker_start(on_start);
        self
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn bind<A: std::net::ToSocketAddrs, ResB, BE>(mut self, addr: A) -> std::io::Result<Self>
    where
        I: Service + 'static,
        I::Response: ReadyService + Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>> + 'static,
        <I::Response as Service<Request<RequestExt<RequestBody>>>>::Error: fmt::Debug,

        ResB: Stream<Item = Result<Bytes, BE>> + 'static,
        BE: fmt::Debug + 'static,
    {
        let factory = self.factory.clone();
        let config = self.config;
        self.builder = self.builder.bind("xitca-web", addr, move || {
            let factory = factory();
            HttpServiceBuilder::with_config(factory, config).with_logger()
        })?;

        Ok(self)
    }

    pub fn listen<ResB, BE>(mut self, listener: std::net::TcpListener) -> std::io::Result<Self>
    where
        I: Service + 'static,
        I::Response: ReadyService + Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>> + 'static,
        <I::Response as Service<Request<RequestExt<RequestBody>>>>::Error: fmt::Debug,

        ResB: Stream<Item = Result<Bytes, BE>> + 'static,
        BE: fmt::Debug + 'static,
    {
        let factory = self.factory.clone();
        let config = self.config;
        self.builder = self.builder.listen("xitca-web", listener, move || {
            let factory = factory();
            HttpServiceBuilder::with_config(factory, config).with_logger()
        });

        Ok(self)
    }

    #[cfg(feature = "openssl")]
    pub fn bind_openssl<A: std::net::ToSocketAddrs, ResB, BE>(
        mut self,
        addr: A,
        mut builder: openssl_crate::ssl::SslAcceptorBuilder,
    ) -> std::io::Result<Self>
    where
        I: Service + 'static,
        I::Response: ReadyService + Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>> + 'static,
        <I::Response as Service<Request<RequestExt<RequestBody>>>>::Error: fmt::Debug,

        ResB: Stream<Item = Result<Bytes, BE>> + 'static,
        BE: fmt::Debug + 'static,
    {
        let factory = self.factory.clone();
        let config = self.config;

        const H11: &[u8] = b"\x08http/1.1";

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

        self.builder = self.builder.bind("xitca-web-openssl", addr, move || {
            let factory = factory();
            HttpServiceBuilder::with_config(factory, config)
                .openssl(acceptor.clone())
                .with_logger()
        })?;

        Ok(self)
    }

    #[cfg(feature = "rustls")]
    pub fn bind_rustls<A: std::net::ToSocketAddrs, ResB, BE>(
        mut self,
        addr: A,
        #[cfg_attr(not(all(feature = "http1", feature = "http2")), allow(unused_mut))]
        mut config: rustls_crate::ServerConfig,
    ) -> std::io::Result<Self>
    where
        I: Service + 'static,
        I::Response: ReadyService + Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>> + 'static,
        <I::Response as Service<Request<RequestExt<RequestBody>>>>::Error: fmt::Debug,

        ResB: Stream<Item = Result<Bytes, BE>> + 'static,
        BE: fmt::Debug + 'static,
    {
        let factory = self.factory.clone();
        let service_config = self.config;

        #[cfg(feature = "http2")]
        config.alpn_protocols.push("h2".into());

        #[cfg(feature = "http1")]
        config.alpn_protocols.push("http/1.1".into());

        let config = std::sync::Arc::new(config);

        self.builder = self.builder.bind("xitca-web-rustls", addr, move || {
            let factory = factory();
            HttpServiceBuilder::with_config(factory, service_config)
                .rustls(config.clone())
                .with_logger()
        })?;

        Ok(self)
    }

    #[cfg(unix)]
    pub fn bind_unix<P: AsRef<std::path::Path>, ResB, BE>(mut self, path: P) -> std::io::Result<Self>
    where
        I: Service + 'static,
        I::Response: ReadyService + Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>> + 'static,
        <I::Response as Service<Request<RequestExt<RequestBody>>>>::Error: fmt::Debug,

        ResB: Stream<Item = Result<Bytes, BE>> + 'static,
        BE: fmt::Debug + 'static,
    {
        let factory = self.factory.clone();
        let config = self.config;

        self.builder = self.builder.bind_unix("xitca-web", path, move || {
            let factory = factory();
            HttpServiceBuilder::with_config(factory, config).with_logger()
        })?;

        Ok(self)
    }

    pub fn run(self) -> ServerFuture {
        self.builder.build()
    }

    fn mutate_const_generic<const HEADER_LIMIT2: usize, const READ_BUF_LIMIT2: usize, const WRITE_BUF_LIMIT2: usize>(
        self,
    ) -> HttpServer<F, HEADER_LIMIT2, READ_BUF_LIMIT2, WRITE_BUF_LIMIT2> {
        HttpServer {
            factory: self.factory,
            builder: self.builder,
            config: self
                .config
                .mutate_const_generic::<HEADER_LIMIT2, READ_BUF_LIMIT2, WRITE_BUF_LIMIT2>(),
        }
    }
}
