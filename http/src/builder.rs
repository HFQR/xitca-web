use std::{fmt, future::Future, marker::PhantomData};

use futures_core::Stream;
use xitca_io::{
    io::AsyncIo,
    net::{Stream as ServerStream, TcpStream},
};
use xitca_service::{ServiceFactory, ServiceFactoryExt, TransformFactory};

use super::{
    body::{RequestBody, ResponseBody},
    bytes::Bytes,
    config::{HttpServiceConfig, DEFAULT_HEADER_LIMIT, DEFAULT_READ_BUF_LIMIT, DEFAULT_WRITE_BUF_LIMIT},
    error::{BodyError, HttpServiceError},
    expect::ExpectHandler,
    http::Response,
    request::Request,
    service::HttpService,
    tls,
    util::middleware::Logger,
    version::AsVersion,
};

// marker type for separate HttpServerBuilders' ServiceFactory implement with specialized trait
// method.
#[doc(hidden)]
pub(crate) mod marker {
    pub struct Http;
    #[cfg(feature = "http1")]
    pub struct Http1;
    #[cfg(feature = "http2")]
    pub struct Http2;
}

/// HttpService Builder type.
/// Take in generic types of ServiceFactory for http and tls.
pub struct HttpServiceBuilder<
    V,
    St,
    F,
    FE,
    FA,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> {
    pub(crate) factory: F,
    pub(crate) expect: FE,
    pub(crate) tls_factory: FA,
    pub(crate) config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    pub(crate) _body: PhantomData<(V, St)>,
}

impl<F>
    HttpServiceBuilder<
        marker::Http,
        ServerStream,
        F,
        ExpectHandler<F>,
        tls::NoOpTlsAcceptorService,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    >
{
    /// Construct a new Service Builder with given service factory and default configuration.
    pub const fn new(factory: F) -> Self {
        Self::with_config(factory, HttpServiceConfig::new())
    }

    /// Construct a new Service Builder with given service factory and configuration
    pub const fn with_config<const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>(
        factory: F,
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    ) -> HttpServiceBuilder<
        marker::Http,
        ServerStream,
        F,
        ExpectHandler<F>,
        tls::NoOpTlsAcceptorService,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    > {
        HttpServiceBuilder {
            factory,
            expect: ExpectHandler::new(),
            tls_factory: tls::NoOpTlsAcceptorService,
            config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http1")]
    /// Construct a new Http/1 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h1::RequestBody>` as Request type.
    /// This is a request type specific for Http/1 request body.
    pub fn h1<ResB, E>(
        factory: F,
    ) -> HttpServiceBuilder<
        marker::Http1,
        TcpStream,
        F,
        ExpectHandler<F>,
        tls::NoOpTlsAcceptorService,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    >
    where
        F: ServiceFactory<Request<super::h1::RequestBody>, Response = Response<ResponseBody<ResB>>>,
        F::Service: 'static,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        HttpServiceBuilder {
            factory,
            expect: ExpectHandler::new(),
            tls_factory: tls::NoOpTlsAcceptorService,
            config: HttpServiceConfig::default(),
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http2")]
    /// Construct a new Http/2 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h2::RequestBody>` as Request type.
    /// This is a request type specific for Http/2 request body.
    pub fn h2<ResB, E>(
        factory: F,
    ) -> HttpServiceBuilder<
        marker::Http2,
        TcpStream,
        F,
        (),
        tls::NoOpTlsAcceptorService,
        DEFAULT_HEADER_LIMIT,
        DEFAULT_READ_BUF_LIMIT,
        DEFAULT_WRITE_BUF_LIMIT,
    >
    where
        F: ServiceFactory<Request<super::h2::RequestBody>, Response = Response<ResponseBody<ResB>>>,
        F::Service: 'static,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        HttpServiceBuilder {
            factory,
            expect: (),
            tls_factory: tls::NoOpTlsAcceptorService,
            config: HttpServiceConfig::default(),
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http3")]
    /// Construct a new Http/3 ServiceBuilder.
    ///
    /// Note factory type F ues `Request<h3::RequestBody>` as Request type.
    /// This is a request type specific for Http/3 request body.
    pub fn h3<ResB, E>(factory: F) -> super::h3::H3ServiceBuilder<F>
    where
        F: ServiceFactory<Request<super::h3::RequestBody>, Response = Response<ResponseBody<ResB>>>,
        F::Service: 'static,

        ResB: Stream<Item = Result<Bytes, E>> + 'static,
        E: 'static,
        BodyError: From<E>,
    {
        super::h3::H3ServiceBuilder::new(factory)
    }
}

impl<V, St, F, FE, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    HttpServiceBuilder<V, St, F, FE, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
{
    pub fn config<const HEADER_LIMIT_2: usize, const READ_BUF_LIMIT_2: usize, const WRITE_BUF_LIMIT_2: usize>(
        self,
        config: HttpServiceConfig<HEADER_LIMIT_2, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT_2>,
    ) -> HttpServiceBuilder<V, St, F, FE, FA, HEADER_LIMIT_2, READ_BUF_LIMIT_2, WRITE_BUF_LIMIT_2> {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            tls_factory: self.tls_factory,
            config,
            _body: PhantomData,
        }
    }

    #[cfg(feature = "http1")]
    pub fn expect<FE2, ReqB, ResB>(
        self,
        expect: FE2,
    ) -> HttpServiceBuilder<V, St, F, FE2, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    where
        FE2: ServiceFactory<Request<ReqB>, Response = Request<ResB>>,
        FE2::Service: 'static,
    {
        HttpServiceBuilder {
            factory: self.factory,
            expect,
            tls_factory: self.tls_factory,
            config: self.config,
            _body: PhantomData,
        }
    }

    /// Pass a tls service factory to Builder and use it to handle tls handling with
    /// Http/1 and Http/2.
    pub fn with_tls<TlsF>(
        self,
        tls_factory: TlsF,
    ) -> HttpServiceBuilder<V, St, F, FE, TlsF, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT> {
        HttpServiceBuilder {
            factory: self.factory,
            expect: self.expect,
            tls_factory,
            config: self.config,
            _body: PhantomData,
        }
    }

    /// Finish builder with default logger.
    ///
    /// Would consume input.
    pub fn with_logger<Req>(self) -> TransformFactory<Self, Logger>
    where
        Self: ServiceFactory<Req>,
        <Self as ServiceFactory<Req>>::Error: fmt::Debug,
    {
        self.transform(Logger::new())
    }

    #[cfg(feature = "openssl")]
    pub fn openssl(
        self,
        acceptor: tls::openssl::TlsAcceptor,
    ) -> HttpServiceBuilder<V, St, F, FE, tls::openssl::TlsAcceptorService, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::openssl::TlsAcceptorService::new(acceptor))
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(
        self,
        config: tls::rustls::RustlsConfig,
    ) -> HttpServiceBuilder<V, St, F, FE, tls::rustls::TlsAcceptorService, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
    {
        self.with_tls(tls::rustls::TlsAcceptorService::new(config))
    }

    #[cfg(feature = "native-tls")]
    pub fn native_tls(
        self,
        acceptor: tls::native_tls::TlsAcceptor,
    ) -> HttpServiceBuilder<
        V,
        St,
        F,
        FE,
        tls::native_tls::TlsAcceptorService,
        HEADER_LIMIT,
        READ_BUF_LIMIT,
        WRITE_BUF_LIMIT,
    > {
        self.with_tls(tls::native_tls::TlsAcceptorService::new(acceptor))
    }
}

impl<F, Arg, ResB, E, FE, FA, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    ServiceFactory<ServerStream, Arg>
    for HttpServiceBuilder<marker::Http, ServerStream, F, FE, FA, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    F: ServiceFactory<Request<RequestBody>, Arg, Response = Response<ResponseBody<ResB>>>,
    F::Service: 'static,
    F::Error: fmt::Debug,

    // TODO: use a meaningful config.
    FE: ServiceFactory<Request<RequestBody>, Response = Request<RequestBody>>,
    FE::Service: 'static,

    FA: ServiceFactory<TcpStream>,
    FA::Service: 'static,
    FA::Response: AsyncIo + AsVersion,

    HttpServiceError<F::Error>: From<FA::Error>,
    F::Error: From<FE::Error>,

    ResB: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    type Response = ();
    type Error = HttpServiceError<F::Error>;
    type Service =
        HttpService<F::Service, RequestBody, FE::Service, FA::Service, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let expect = self.expect.new_service(());
        let service = self.factory.new_service(arg);
        let tls_acceptor = self.tls_factory.new_service(());
        let config = self.config;

        async move {
            // TODO: clean up error types.
            let expect = expect
                .await
                .map_err(|e| HttpServiceError::<F::Error>::Service(e.into()))?;
            let service = service.await.map_err(HttpServiceError::<F::Error>::Service)?;
            let tls_acceptor = tls_acceptor.await?;
            Ok(HttpService::new(config, service, expect, tls_acceptor))
        }
    }
}
