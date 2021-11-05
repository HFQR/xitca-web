use xitca_client::Client;
use xitca_http::{
    body::ResponseBody,
    h1,
    http::{header, Method, Request, Response},
};
use xitca_io::bytes::{Bytes, BytesMut};
use xitca_service::fn_service;
use xitca_test::{test_h1_server, Error};

#[tokio::test]
async fn h1_get() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::new();

    let res = c.get(&server_url)?.send().await?;
    assert_eq!(res.status().as_u16(), 200);
    let body = res.string().await?;
    assert_eq!("GET Response", body);

    for _ in 0..3 {
        let mut req = c.post(&server_url)?.body(Bytes::from("Hello,World!"));
        req.headers_mut()
            .insert(header::CONTENT_TYPE, header::HeaderValue::from_static("text/plain"));
        let res = req.send().await?;
        assert_eq!(res.status().as_u16(), 200);
        let body = res.string().await?;
        assert_eq!("Hello,World!", body);
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_post() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 * 1024 {
            body.extend_from_slice(b"Hello,World!");
        }
        let body_len = body.len();

        let mut req = c.post(&server_url)?.body(body);
        req.headers_mut()
            .insert(header::CONTENT_TYPE, header::HeaderValue::from_static("text/plain"));
        let res = req.send().await?;
        assert_eq!(res.status().as_u16(), 200);
        let body = res.limit::<{ 12 * 1024 * 1024 }>().string().await?;
        assert_eq!(body.len(), body_len);
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_partial_body_read() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let server_url = format!("http://{}/drop_body", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 * 1024 {
            body.extend_from_slice(b"Hello,World!");
        }

        let mut req = c.post(&server_url)?.body(body);

        req.headers_mut()
            .insert(header::CONTENT_TYPE, header::HeaderValue::from_static("text/plain"));

        let res = req.send().await?;

        assert_eq!(res.status().as_u16(), 200);
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

async fn handle(req: Request<h1::RequestBody>) -> Result<Response<ResponseBody>, Error> {
    // Some yield for testing h1 dispatcher's concurrent future handling.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => Ok(Response::new(Bytes::from("GET Response").into())),
        (&Method::POST, "/") => {
            let length = req.headers().get(header::CONTENT_LENGTH).unwrap().clone();
            let ty = req.headers().get(header::CONTENT_TYPE).unwrap().clone();
            let body = req.into_body();
            let mut res = Response::new(ResponseBody::stream(Box::pin(body) as _));

            res.headers_mut().insert(header::CONTENT_LENGTH, length);
            res.headers_mut().insert(header::CONTENT_TYPE, ty);

            Ok(res)
        }
        // drop request body. server should close connection afterwards.
        (&Method::POST, "/drop_body") => Ok(Response::new(Bytes::new().into())),
        _ => todo!(),
    }
}
