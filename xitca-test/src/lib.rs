use std::{
    error, fmt,
    future::Future,
    io,
    net::{SocketAddr, TcpListener},
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::Stream;
use xitca_http::{
    body::ResponseBody,
    error::BodyError,
    h1, h2,
    http::{Request, Response},
    HttpServiceBuilder,
};
use xitca_io::{bytes::Bytes, net::TcpStream};
use xitca_server::{net::FromStream, Builder, ServerFuture, ServerHandle};
use xitca_service::ServiceFactory;

pub type Error = Box<dyn error::Error + Send + Sync>;

/// A general test server for any given service type that accept the connection from
/// xitca-server
pub fn test_server<F, T, Req>(factory: F) -> Result<TestServerHandle, Error>
where
    F: Fn() -> T + Send + Clone + 'static,
    T: ServiceFactory<Req, Config = ()>,
    Req: FromStream + Send + 'static,
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
    F: Fn() -> I + Send + Clone + 'static,
    I: ServiceFactory<Request<h1::RequestBody>, Response = Response<ResponseBody<B>>, Config = (), InitError = ()>
        + 'static,
    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    test_server::<_, _, TcpStream>(move || {
        let f = factory();
        HttpServiceBuilder::h1(f)
    })
}

/// A specialized http/2 server on top of [test_server]
pub fn test_h2_server<F, I, B, E>(factory: F) -> Result<TestServerHandle, Error>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: ServiceFactory<Request<h2::RequestBody>, Response = Response<ResponseBody<B>>, Config = (), InitError = ()>
        + 'static,
    I::Error: fmt::Debug,
    B: Stream<Item = Result<Bytes, E>> + 'static,
    E: 'static,
    BodyError: From<E>,
{
    test_server::<_, _, TcpStream>(move || {
        let f = factory();
        HttpServiceBuilder::h2(f)
    })
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
