#[cfg(feature = "io-uring")]
use {
    core::{convert::Infallible, net::SocketAddr},
    futures_util::stream::StreamExt,
    xitca_client::Client,
    xitca_http::{
        body::Once,
        bytes::Bytes,
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
    async fn handler(req: Request<RequestExt<h2::RequestBodyV2>>) -> Result<Response<Once<Bytes>>, Infallible> {
        let (_, ext) = req.into_parts();
        let (_, mut body) = ext.replace_body(());

        let mut s = Vec::new();

        while let Some(chunk) = body.next().await {
            let chunk = chunk.unwrap();
            s.extend_from_slice(chunk.as_ref());
        }

        Ok(Response::new(Bytes::from(s).into()))
    }

    let (tx2, mut rx2) = tokio::sync::mpsc::unbounded_channel();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let service = fn_service(
            move |((stream, _), _): ((TcpStream, SocketAddr), tokio_util::sync::CancellationToken)| {
                let tx2 = tx2.clone();
                async move {
                    h2::run(stream, &fn_service(handler).call(()).now_or_panic().unwrap())
                        .await
                        .map(|_| {
                            let _ = tx2.send(());
                        })
                }
            },
        );
        let server = xitca_server::Builder::new()
            .bind("qa", "localhost:8080", service)?
            .build();

        tx.send(()).unwrap();

        server.wait()
    });

    rx.recv().unwrap();

    let c = Client::new();

    let mut req = c.post("https://localhost:8080/");

    req.headers_mut().insert("foo", "bar".parse().unwrap());

    let mut req_body = String::new();

    for _ in 0..1024 * 1024 {
        req_body.push_str("hello,world!");
    }

    let res = req
        .version(Version::HTTP_2)
        .body(req_body.clone())
        .send()
        .await
        .unwrap();

    assert!(res.status().is_success());

    let body = res.string().await.unwrap();

    assert_eq!(req_body, body);

    drop(c);

    rx2.recv().await;
}
