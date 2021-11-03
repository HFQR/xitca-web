use std::{
    error,
    net::{SocketAddr, TcpListener},
};

use xitca_http::{
    body::ResponseBody,
    h1,
    http::{Request, Response},
    HttpServiceBuilder,
};
use xitca_io::net::TcpStream;
use xitca_server::{Builder, ServerFuture};
use xitca_service::ServiceFactory;

type Error = Box<dyn error::Error + Send + Sync>;

pub fn test_h1_server<F, I>(factory: F) -> Result<(SocketAddr, ServerFuture), Error>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: ServiceFactory<Request<h1::RequestBody>, Response = Response<ResponseBody>, Config = (), InitError = ()>
        + 'static,
{
    let lst = TcpListener::bind("127.0.0.1:0")?;

    let addr = lst.local_addr()?;

    let server = Builder::new()
        .listen::<_, _, TcpStream>("websocket", lst, move || {
            let factory = factory();
            HttpServiceBuilder::h1(factory)
        })?
        .build();

    Ok((addr, server))
}
