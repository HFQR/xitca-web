use std::{
    error, fmt, fs,
    future::Future,
    io,
    net::SocketAddr,
    net::TcpListener,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures_util::Stream;
use tokio_util::sync::CancellationToken;
use xitca_http::{
    body::ResponseBody,
    config::HttpServiceConfig,
    h1, h2, h3,
    http::{Request, RequestExt, Response},
    HttpServiceBuilder,
};
use xitca_io::{
    bytes::Bytes,
    net::{Stream as NetStream, TcpStream},
};
use xitca_server::{Builder, ServerFuture, ServerHandle};
use xitca_service::{ready::ReadyService, Service, ServiceExt};

pub type Error = Box<dyn error::Error + Send + Sync>;

type HResponse<B> = Response<ResponseBody<B>>;

/// A general test server for any given service type that accept the connection from
/// xitca-server
pub fn test_server<T, Req>(service: T) -> Result<TestServerHandle, Error>
where
    T: Service + Send + Sync + 'static,
    T::Response: ReadyService + Service<(Req, CancellationToken)>,
    Req: TryFrom<NetStream> + 'static,
{
    let lst = TcpListener::bind("127.0.0.1:0")?;

    let addr = lst.local_addr()?;

    let handle = Builder::new()
        .worker_threads(1)
        .server_threads(1)
        .disable_signal()
        .listen::<_, _, _, Req>("test_server", lst, service)
        .build();

    Ok(TestServerHandle { addr, handle })
}

/// A specialized http/1 server on top of [test_server]
pub fn test_h1_server<T, B, E>(service: T) -> Result<TestServerHandle, Error>
where
    T: Service + Send + Sync + 'static,
    T::Response: ReadyService + Service<Request<RequestExt<h1::RequestBody>>, Response = HResponse<B>> + 'static,
    <T::Response as Service<Request<RequestExt<h1::RequestBody>>>>::Error: fmt::Debug,
    T::Error: error::Error + 'static,
    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: fmt::Debug + 'static,
{
    #[cfg(not(feature = "io-uring"))]
    {
        test_server::<_, (TcpStream, SocketAddr)>(service.enclosed(HttpServiceBuilder::h1()))
    }

    #[cfg(feature = "io-uring")]
    {
        test_server::<_, (xitca_io::net::io_uring::TcpStream, SocketAddr)>(
            service.enclosed(HttpServiceBuilder::h1().io_uring()),
        )
    }
}

/// A specialized http/2 server on top of [test_server]
pub fn test_h2_server<T, B, E>(service: T) -> Result<TestServerHandle, Error>
where
    T: Service + Send + Sync + 'static,
    T::Response: ReadyService + Service<Request<RequestExt<h2::RequestBody>>, Response = HResponse<B>> + 'static,
    <T::Response as Service<Request<RequestExt<h2::RequestBody>>>>::Error: fmt::Debug,
    T::Error: error::Error + 'static,
    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: fmt::Debug + 'static,
{
    test_server::<_, (TcpStream, SocketAddr)>(
        service.enclosed(
            HttpServiceBuilder::h2().config(
                HttpServiceConfig::new()
                    .request_head_timeout(Duration::from_millis(500))
                    .tls_accept_timeout(Duration::from_millis(500))
                    .keep_alive_timeout(Duration::from_millis(500)),
            ),
        ),
    )
}

/// A specialized http/3 server
pub fn test_h3_server<T, B, E>(service: T) -> Result<TestServerHandle, Error>
where
    T: Service + Send + Sync + 'static,
    T::Response: ReadyService + Service<Request<RequestExt<h3::RequestBody>>, Response = HResponse<B>> + 'static,
    <T::Response as Service<Request<RequestExt<h3::RequestBody>>>>::Error: fmt::Debug,
    T::Error: error::Error + 'static,
    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: fmt::Debug + 'static,
{
    let addr = std::net::UdpSocket::bind("127.0.0.1:0")?.local_addr()?;

    let key = fs::read("../examples/cert/key.pem")?;
    let cert = fs::read("../examples/cert/cert.pem")?;

    let key = rustls_pemfile::pkcs8_private_keys(&mut &*key).next().unwrap().unwrap();
    let key = h3_quinn::quinn::rustls::pki_types::PrivateKeyDer::from(key);

    let cert = rustls_pemfile::certs(&mut &*cert).collect::<Result<_, _>>().unwrap();

    let mut config = h3_quinn::quinn::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert, key)?;

    config.alpn_protocols = vec![b"h3".to_vec(), b"h3-29".to_vec(), b"h3-28".to_vec(), b"h3-27".to_vec()];

    let config = h3_quinn::quinn::crypto::rustls::QuicServerConfig::try_from(config).unwrap();

    let config = h3_quinn::quinn::ServerConfig::with_crypto(std::sync::Arc::new(config));

    let listener = xitca_io::net::QuicListenerBuilder::new(addr, config);

    let handle = Builder::new()
        .worker_threads(1)
        .server_threads(1)
        .disable_signal()
        .listen("test_server", listener, service.enclosed(HttpServiceBuilder::h3()))
        .build();

    Ok(TestServerHandle { addr, handle })
}

pub struct TestServerHandle {
    addr: SocketAddr,
    handle: ServerFuture,
}

impl TestServerHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn ip_port_string(&self) -> String {
        format!("{}:{}", self.addr.ip(), self.addr.port())
    }

    pub fn try_handle(&mut self) -> io::Result<ServerHandle> {
        self.handle.handle()
    }
}

impl Future for TestServerHandle {
    type Output = <ServerFuture as Future>::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().handle).poll(cx)
    }
}
