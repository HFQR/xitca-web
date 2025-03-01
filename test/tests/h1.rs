use futures_util::StreamExt;
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

use xitca_client::Client;
use xitca_http::{
    body::{BoxBody, ResponseBody},
    bytes::{Bytes, BytesMut},
    h1,
    http::{
        header::{self, HeaderValue, CONNECTION},
        Method, Request, RequestExt, Response, Version,
    },
};
use xitca_service::fn_service;
use xitca_test::{test_h1_server, Error};

#[tokio::test]
async fn h1_get() -> Result<(), Error> {
    let mut handle = test_h1_server(fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut res = c.get(&server_url).version(Version::HTTP_11).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(!res.can_close_connection());
        let body = res.string().await?;
        assert_eq!("GET Response", body);
    }

    handle.try_handle()?.stop(true);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_get_without_body_reading() -> Result<(), Error> {
    let mut handle = test_h1_server(fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::builder().set_pool_capacity(1).finish();

    let mut res = c.get(&server_url).version(Version::HTTP_11).send().await?;
    assert_eq!(res.status().as_u16(), 200);
    assert!(!res.can_close_connection());

    // drop the response body without reading it.
    drop(res);

    let mut res = c.get(&server_url).version(Version::HTTP_11).send().await?;
    assert_eq!(res.status().as_u16(), 200);
    assert!(!res.can_close_connection());
    let body = res.string().await?;
    assert_eq!("GET Response", body);

    handle.try_handle()?.stop(false);
    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_head() -> Result<(), Error> {
    let mut handle = test_h1_server(fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut res = c.head(&server_url).version(Version::HTTP_11).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(!res.can_close_connection());
        let body = res.string().await?;
        assert_eq!("", body);
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_post() -> Result<(), Error> {
    let mut handle = test_h1_server(fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 {
            body.extend_from_slice(b"Hello,World!");
        }
        let body_len = body.len();

        let mut res = c.post(&server_url).version(Version::HTTP_11).text(body).send().await?;
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
    let mut handle = test_h1_server(fn_service(handle))?;

    let server_url = format!("http://{}/drop_body", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 * 1024 {
            body.extend_from_slice(b"Hello,World!");
        }

        let mut res = c.post(&server_url).version(Version::HTTP_11).text(body).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(res.can_close_connection());
    }

    handle.try_handle()?.stop(true);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_partial_body_read() -> Result<(), Error> {
    let mut handle = test_h1_server(fn_service(handle))?;

    let server_url = format!("http://{}/partial_read", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 * 1024 {
            body.extend_from_slice(b"Hello,World!");
        }

        let mut res = c.post(&server_url).version(Version::HTTP_11).text(body).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(res.can_close_connection());
    }

    handle.try_handle()?.stop(true);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h1_close_connection() -> Result<(), Error> {
    let mut handle = test_h1_server(fn_service(handle))?;

    let server_url = format!("http://{}/close_connection", handle.ip_port_string());

    let c = Client::new();

    let mut res = c.get(&server_url).version(Version::HTTP_11).send().await?;
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
    let mut handle = test_h1_server(fn_service(handle))?;

    let server_url = format!("http://{}/", handle.ip_port_string());

    let c = Client::new();

    let mut req = c.get(&server_url).version(Version::HTTP_11);

    let body = vec![*b"H".first().unwrap(); 512 * 1024];
    req.headers_mut()
        .insert("large-header", HeaderValue::try_from(body).unwrap());

    let res = req.send().await?;
    assert_eq!(res.status().as_u16(), 200);
    let _ = res.body().await;

    let mut req = c.get(&server_url).version(Version::HTTP_11);

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
    let mut handle = test_h1_server(fn_service(handle))?;

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

#[tokio::test]
async fn h1_shutdown_during_request() -> Result<(), Error> {
    let mut handle = test_h1_server(fn_service(handle))?;

    let mut stream = TcpStream::connect(handle.addr())?;

    let mut buf = [0; 128];

    // write first half of the request.
    stream.write_all(SLEEP_GET_REQ_PART_1)?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // shutdown server.
    handle.try_handle()?.stop(true);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // write second half of the request.
    stream.write_all(SLEEP_GET_REQ_PART_2)?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // write second half of the request.
    stream.write_all(SLEEP_GET_REQ_PART_3)?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // read response.
    loop {
        let n = stream.read(&mut buf)?;

        if n == 0 {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "unexpected eof",
            )))?;
        }

        // it should be chunked so it ends with 0\r\n\r\n
        if buf[..n].ends_with(b"0\r\n\r\n") {
            break;
        }
    }

    handle.await?;

    Ok(())
}

async fn handle(req: Request<RequestExt<h1::RequestBody>>) -> Result<Response<ResponseBody>, Error> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") | (&Method::HEAD, "/") => Ok(Response::new(Bytes::from("GET Response").into())),
        (&Method::POST, "/") => {
            let length = req.headers().get(header::CONTENT_LENGTH).unwrap().clone();
            let ty = req.headers().get(header::CONTENT_TYPE).unwrap().clone();

            let body = req.into_body();
            let mut res = Response::new(ResponseBody::stream(BoxBody::new(body)));

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
        (&Method::GET, "/sleep") => {
            tokio::time::sleep(Duration::from_millis(200)).await;

            Ok(Response::new(ResponseBody::stream(BoxBody::new(req.into_body()))))
        }
        _ => todo!(),
    }
}

const SIMPLE_GET_REQ: &[u8] = b"GET / HTTP/1.1\r\ncontent-length: 0\r\n\r\n";

const SLEEP_GET_REQ_PART_1: &[u8] = b"GET /sleep HTTP/1.1\r\n";
const SLEEP_GET_REQ_PART_2: &[u8] = b"content-length: 10\r\n\r\n01";
const SLEEP_GET_REQ_PART_3: &[u8] = b"23456789";
