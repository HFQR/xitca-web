#[cfg(feature = "io-uring")]
use {
    std::convert::Infallible,
    std::net::SocketAddr,
    xitca_client::Client,
    xitca_http::{
        h2,
        http::{Request, RequestExt, Response, Version},
    },
    xitca_io::net::io_uring::TcpStream,
    xitca_service::{fn_service, Service},
    xitca_unsafe_collection::futures::NowOrPanic,
};

#[cfg(feature = "io-uring")]
#[tokio::test]
async fn h2_v2_get() {
    async fn handler(req: Request<RequestExt<h2::RequestBodyV2>>) -> Result<Response<()>, Infallible> {
        dbg!(req.headers());
        Ok(Response::new(()))
    }

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
