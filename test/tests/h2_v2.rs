#[cfg(feature = "io-uring")]
use {
    futures_util::stream::StreamExt,
    std::{convert::Infallible, net::SocketAddr},
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
async fn h2_v2_post() {
    async fn handler(req: Request<RequestExt<h2::RequestBodyV2>>) -> Result<Response<()>, Infallible> {
        let (_, ext) = req.into_parts();
        let (_, mut body) = ext.replace_body(());

        let mut s = String::new();

        while let Some(chunk) = body.next().await {
            let chunk = chunk.unwrap();
            s.push_str(std::str::from_utf8(chunk.as_ref()).unwrap());
        }

        Ok(Response::new(()))
    }

    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let service = fn_service(|(stream, _): (TcpStream, SocketAddr)| {
            h2::run(stream, fn_service(handler).call(()).now_or_panic().unwrap())
        });
        let server = xitca_server::Builder::new()
            .bind("qa", "localhost:8080", service)?
            .build();

        tx.send(()).unwrap();

        server.wait()
    });

    rx.recv().unwrap();

    let c = Client::new();

    let mut req = c.post("https://localhost:8080/").unwrap();

    req.headers_mut().insert("foo", "bar".parse().unwrap());

    let res = req.version(Version::HTTP_2).body("hello,world!").send().await.unwrap();

    assert!(res.status().is_success());
}
