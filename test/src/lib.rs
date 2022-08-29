use std::{
    error, fmt, fs,
    future::Future,
    io,
    net::{SocketAddr, TcpListener},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures_util::Stream;
use xitca_http::{
    body::ResponseBody, config::HttpServiceConfig, h1, h2, h3, http::Response, HttpServiceBuilder, Request,
};
use xitca_io::{
    bytes::Bytes,
    net::{Stream as NetStream, TcpStream},
};
use xitca_server::{Builder, ServerFuture, ServerHandle};
use xitca_service::{ready::ReadyService, BuildService, Service};

pub type Error = Box<dyn error::Error + Send + Sync>;

type HResponse<B> = Response<ResponseBody<B>>;

/// A general test server for any given service type that accept the connection from
/// xitca-server
pub fn test_server<F, T, Req>(factory: F) -> Result<TestServerHandle, Error>
where
    F: Fn() -> T + Send + Sync + 'static,
    T: BuildService,
    T::Service: ReadyService + Service<Req>,
    Req: From<NetStream> + Send + 'static,
{
    let lst = TcpListener::bind("127.0.0.1:0")?;

    let addr = lst.local_addr()?;

    let handle = Builder::new()
        .worker_threads(1)
        .server_threads(1)
        .disable_signal()
        .listen::<_, _, Req>("test_server", lst, factory)?
        .build();

    Ok(TestServerHandle { addr, handle })
}

/// A specialized http/1 server on top of [test_server]
pub fn test_h1_server<F, I, B, E>(factory: F) -> Result<TestServerHandle, Error>
where
    F: Fn() -> I + Send + Sync + 'static,
    I: BuildService + 'static,
    I::Service: ReadyService + Service<Request<h1::RequestBody>, Response = HResponse<B>> + 'static,
    <I::Service as Service<Request<h1::RequestBody>>>::Error: fmt::Debug,
    I::Error: error::Error + 'static,
    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: fmt::Debug + 'static,
{
    test_server::<_, _, TcpStream>(move || {
        let f = factory();
        HttpServiceBuilder::h1(f)
    })
}

/// A specialized http/2 server on top of [test_server]
pub fn test_h2_server<F, I, B, E>(factory: F) -> Result<TestServerHandle, Error>
where
    F: Fn() -> I + Send + Sync + 'static,
    I: BuildService + 'static,
    I::Service: ReadyService + Service<Request<h2::RequestBody>, Response = HResponse<B>> + 'static,
    <I::Service as Service<Request<h2::RequestBody>>>::Error: fmt::Debug,
    I::Error: error::Error + 'static,
    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: fmt::Debug + 'static,
{
    test_server::<_, _, TcpStream>(move || {
        let f = factory();
        let config = HttpServiceConfig::new()
            .first_request_timeout(Duration::from_millis(500))
            .tls_accept_timeout(Duration::from_millis(500))
            .keep_alive_timeout(Duration::from_millis(500));
        HttpServiceBuilder::h2(f).config(config)
    })
}

/// A specialized http/3 server
pub fn test_h3_server<F, I, B, E>(factory: F) -> Result<TestServerHandle, Error>
where
    F: Fn() -> I + Send + Sync + 'static,
    I: BuildService + 'static,
    I::Service: ReadyService + Service<Request<h3::RequestBody>, Response = HResponse<B>> + 'static,
    <I::Service as Service<Request<h3::RequestBody>>>::Error: fmt::Debug,
    I::Error: error::Error + 'static,
    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: fmt::Debug + 'static,
{
    let addr = std::net::UdpSocket::bind("127.0.0.1:0")?.local_addr()?;

    let key = fs::read("../examples/cert/key.pem")?;
    let cert = fs::read("../examples/cert/cert.pem")?;

    let key = rustls_pemfile::pkcs8_private_keys(&mut &*key)?.remove(0);
    let key = rustls::PrivateKey(key);

    let cert = rustls_pemfile::certs(&mut &*cert)?
        .into_iter()
        .map(rustls::Certificate)
        .collect();

    let mut config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert, key)?;

    config.alpn_protocols = vec![b"h3".to_vec(), b"h3-29".to_vec(), b"h3-28".to_vec(), b"h3-27".to_vec()];

    let config = h3_quinn::quinn::ServerConfig::with_crypto(std::sync::Arc::new(config));

    let handle = Builder::new()
        .worker_threads(1)
        .server_threads(1)
        .disable_signal()
        .bind_h3("test_server", addr, config, move || {
            let f = factory();
            HttpServiceBuilder::h3(f)
        })?
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
