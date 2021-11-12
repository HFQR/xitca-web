// use std::time::{Duration, Instant};

use futures_util::StreamExt;
// use xitca_client::Client;
use xitca_http::{
    body::ResponseBody,
    bytes::{Bytes, BytesMut},
    h3,
    http::{header, Method, Request, Response},
};
use xitca_service::fn_service;
use xitca_test::{test_h3_server, Error};

#[tokio::test]
async fn h3_get() -> Result<(), Error> {
    let mut handle = test_h3_server(|| fn_service(handle))?;

    // let c = Client::new();
    //
    // for _ in 0..3 {
    //     // TODO: rustls does not support 127.0.0.1 as DNS host name.
    //     let mut res = c
    //         .get("https://localhost:8080/")?
    //         .version(Version::HTTP_3)
    //         .send()
    //         .await?;
    //     assert_eq!(res.status().as_u16(), 200);
    //     assert!(!res.is_close_connection());
    //     let body = res.string().await?;
    //     assert_eq!("GET Response", body);
    // }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

async fn handle(req: Request<h3::RequestBody>) -> Result<Response<ResponseBody>, Error> {
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
