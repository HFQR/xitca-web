use futures_util::StreamExt;
use xitca_client::Client;
use xitca_http::{
    body::ResponseBody,
    bytes::{Bytes, BytesMut},
    h3,
    http::{header, Method, Request, RequestExt, Response, Version},
};
use xitca_service::fn_service;
use xitca_test::{test_h3_server, Error};

#[tokio::test]
async fn h3_get() -> Result<(), Error> {
    let mut handle = test_h3_server(fn_service(handle))?;

    let c = Client::new();
    let server_url = format!("https://localhost:{}/", handle.addr().port());

    for _ in 0..3 {
        let mut res = c.get(&server_url)?.version(Version::HTTP_3).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(!res.can_close_connection());
        let body = res.string().await?;
        assert_eq!("GET Response", body);
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h3_post() -> Result<(), Error> {
    let mut handle = test_h3_server(fn_service(handle))?;

    let c = Client::new();

    let server_url = format!("https://localhost:{}/", handle.addr().port());

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 * 1024 {
            body.extend_from_slice(b"Hello,World!");
        }
        let mut res = c.post(&server_url)?.version(Version::HTTP_3).text(body).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(!res.can_close_connection());
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

async fn handle(req: Request<RequestExt<h3::RequestBody>>) -> Result<Response<ResponseBody>, Error> {
    // Some yield for testing h3 dispatcher's concurrent future handling.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => Ok(Response::new(Bytes::from("GET Response").into())),
        (&Method::POST, "/") => {
            let (parts, mut body) = req.into_parts();

            let length = parts
                .headers
                .get(header::CONTENT_LENGTH)
                .unwrap()
                .to_str()?
                .parse::<usize>()?;

            let mut buf = BytesMut::new();

            while let Some(bytes) = body.next().await {
                buf.extend_from_slice(&bytes?);
            }

            assert_eq!(buf.len(), length);

            Ok(Response::new(Bytes::new().into()))
        }
        _ => todo!(),
    }
}
