use futures_util::StreamExt;
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

use xitca_client::Client;
use xitca_http::{
    body::{BoxStream, ResponseBody},
    bytes::{Bytes, BytesMut},
    h1,
    http::{
        header::{self, HeaderValue, CONNECTION},
        Method, Request, RequestExt, Response,
    },
};
use xitca_service::fn_service;
use xitca_test::{test_h1_server, Error};

#[tokio::test]
async fn h1_get() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut res = c.get(&server_url)?.send().await?;
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
async fn h1_post() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 {
            body.extend_from_slice(b"Hello,World!");
        }
        let body_len = body.len();

        let mut res = c.post(&server_url)?.text(body).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(!res.can_close_connection());
        let body = res.limit::<{ 12 * 1024 }>().string().await?;
        assert_eq!(body.len(), body_len);
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_drop_body_read() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let server_url = format!("http://{}/drop_body", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 * 1024 {
            body.extend_from_slice(b"Hello,World!");
        }

        let mut res = c.post(&server_url)?.text(body).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(res.can_close_connection());
    }

    handle.try_handle()?.stop(true);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_partial_body_read() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let server_url = format!("http://{}/partial_read", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 * 1024 {
            body.extend_from_slice(b"Hello,World!");
        }

        let mut res = c.post(&server_url)?.text(body).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(res.can_close_connection());
    }

    handle.try_handle()?.stop(true);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_close_connection() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let server_url = format!("http://{}/close_connection", handle.ip_port_string());

    let c = Client::new();

    let mut res = c.get(&server_url)?.send().await?;
    assert_eq!(res.status().as_u16(), 200);
    assert!(res.can_close_connection());

    handle.try_handle()?.stop(true);

    handle.await?;

    Ok(())
}

// Request head size is limited by ReadBuf's max size which is 1MB by default.
// If the default setting changed this test must be chagned to reflex it.
#[tokio::test]
async fn h1_request_too_large() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::new();

    let mut req = c.get(&server_url)?;

    let body = vec![*b"H".first().unwrap(); 512 * 1024];
    req.headers_mut()
        .insert("large-header", HeaderValue::try_from(body).unwrap());

    let res = req.send().await?;
    assert_eq!(res.status().as_u16(), 200);
    let _ = res.body().await;

    let mut req = c.get(&server_url)?;

    let body = vec![*b"H".first().unwrap(); 1024 * 1024];
    req.headers_mut()
        .insert("large-header", HeaderValue::try_from(body).unwrap());

    let mut res = req.send().await?;
    assert_eq!(res.status().as_u16(), 431);
    assert!(res.can_close_connection());

    handle.try_handle()?.stop(true);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_keepalive() -> Result<(), Error> {
    let mut handle = test_h1_server(|| fn_service(handle))?;

    let mut stream = TcpStream::connect(handle.addr())?;

    let mut buf = [0; 128];
    for _ in 0..3 {
        stream.write_all(SIMPLE_GET_REQ)?;

        // this is a loop as http1 with io-uring can split the request head and body to two
        // packets through localhost.
        loop {
            let n = stream.read(&mut buf)?;
            if buf[..n].ends_with(b"GET Response") {
                break;
            }
        }
    }

    // default h1 keep alive is 5 seconds.
    // If test_server has a different setting this must be changed accordingly.
    tokio::time::sleep(Duration::from_secs(7)).await;

    // Due to platform difference this block may success or not.
    // Either way it can not get a response from server as it has already close
    // the connection.
    {
        let _ = stream.write_all(SIMPLE_GET_REQ);

        if let Ok(n) = stream.read(&mut buf) {
            assert_eq!(n, 0);
        }
    }

    handle.try_handle()?.stop(true);

    handle.await?;

    Ok(())
}

async fn handle(req: Request<RequestExt<h1::RequestBody>>) -> Result<Response<ResponseBody>, Error> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => Ok(Response::new(Bytes::from("GET Response").into())),
        (&Method::POST, "/") => {
            let length = req.headers().get(header::CONTENT_LENGTH).unwrap().clone();
            let ty = req.headers().get(header::CONTENT_TYPE).unwrap().clone();

            let body = req.into_body();
            let mut res = Response::new(ResponseBody::stream(BoxStream::new(body)));

            res.headers_mut().insert(header::CONTENT_LENGTH, length);
            res.headers_mut().insert(header::CONTENT_TYPE, ty);

            Ok(res)
        }
        // drop request body. server should close connection afterwards.
        (&Method::POST, "/drop_body") => Ok(Response::new(Bytes::new().into())),
        // partial read request body. server should close connection afterwards.
        (&Method::POST, "/partial_read") => {
            let (parts, mut body) = req.into_parts();

            let length = parts
                .headers
                .get(header::CONTENT_LENGTH)
                .unwrap()
                .to_str()?
                .parse::<usize>()?;

            let bytes = body.next().await.unwrap()?;

            assert!(bytes.len() < length);

            Ok(Response::new(Bytes::new().into()))
        }
        (&Method::GET, "/close_connection") => {
            let mut res = Response::new(Bytes::new().into());
            res.headers_mut().insert(CONNECTION, HeaderValue::from_static("close"));
            Ok(res)
        }
        _ => todo!(),
    }
}

const SIMPLE_GET_REQ: &[u8] = b"GET / HTTP/1.1\r\ncontent-length: 0\r\n\r\n";
