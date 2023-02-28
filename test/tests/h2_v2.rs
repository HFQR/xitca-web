use std::convert::Infallible;
use std::net::SocketAddr;

use xitca_client::Client;
use xitca_http::{
    h2,
    http::{Request, RequestExt, Response, Version},
};
use xitca_io::net::TcpStream;
use xitca_service::{fn_service, Service};
use xitca_unsafe_collection::futures::NowOrPanic;

#[tokio::test]
async fn h2_v2_get() {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let factory = || {
            fn_service(|(stream, _): (TcpStream, SocketAddr)| {
                h2::run(stream, fn_service(handler).call(()).now_or_panic().unwrap())
            })
        };
        let server = xitca_server::Builder::new()
            .bind("qa", "localhost:8080", factory)?
            .build();

        tx.send(()).unwrap();

        server.wait()
    });

    rx.recv().unwrap();

    let c = Client::new();

    let mut req = c.get("https://localhost:8080/").unwrap();

    req.headers_mut().insert("foo", "bar".parse().unwrap());

    let res = req.version(Version::HTTP_2).send().await.unwrap();

    assert!(res.status().is_success());
}

async fn handler(req: Request<RequestExt<h2::RequestBodyV2>>) -> Result<Response<()>, Infallible> {
    dbg!(req.headers());
    Ok(Response::new(()))
}
