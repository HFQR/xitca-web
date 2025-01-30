use std::time::{Duration, Instant};

use futures_util::StreamExt;
use xitca_client::Client;
use xitca_http::{
    body::ResponseBody,
    bytes::{Bytes, BytesMut},
    h2,
    http::{header, Method, Request, RequestExt, Response, Version},
};
use xitca_service::fn_service;
use xitca_test::{test_h2_server, Error};

#[tokio::test]
async fn h2_get() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;

    let server_url = format!("https://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut res = c.get(&server_url).version(Version::HTTP_2).send().await?;
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
async fn h2_no_host_header() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;

    let server_url = format!("https://{}/host", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut req = c.get(&server_url).version(Version::HTTP_2);
        req.headers_mut().insert(header::HOST, "localhost".parse().unwrap());

        let mut res = req.send().await?;
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
async fn h2_post() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;

    let server_url = format!("https://{}/", handle.ip_port_string());

    let c = Client::new();

    for _ in 0..3 {
        let mut body = BytesMut::new();
        for _ in 0..1024 * 1024 {
            body.extend_from_slice(b"Hello,World!");
        }
        let mut res = c.post(&server_url).version(Version::HTTP_2).text(body).send().await?;
        assert_eq!(res.status().as_u16(), 200);
        assert!(!res.can_close_connection());
        let _ = res.body().await;
    }

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h2_connect() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;

    let server_url = format!("https://{}/", handle.ip_port_string());

    let c = Client::new();

    let mut tunnel = c
        .connect(&server_url)
        .version(Version::HTTP_2)
        .send()
        .await?
        .into_inner();

    use xitca_io::io::{AsyncIo, Interest};

    use std::io::{Read, Write};

    tunnel.ready(Interest::WRITABLE).await?;

    tunnel.write_all(b"996")?;

    let mut buf = [0; 8];

    tunnel.ready(Interest::READABLE).await?;

    let n = tunnel.read(&mut buf)?;

    assert_eq!(b"996", &buf[..n]);

    core::future::poll_fn(|cx| core::pin::Pin::new(&mut tunnel).poll_shutdown(cx)).await?;

    handle.try_handle()?.stop(false);

    handle.await?;

    Ok(())
}

#[tokio::test]
async fn h2_keepalive() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;

    let server_url = format!("https://{}/", handle.ip_port_string());

    let (tx, rx) = std::sync::mpsc::sync_channel::<()>(1);

    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let c = Client::new();

                let mut res = c.get(&server_url).version(Version::HTTP_2).send().await?;
                assert_eq!(res.status().as_u16(), 200);
                assert!(!res.can_close_connection());
                let body = res.string().await?;
                assert_eq!("GET Response", body);

                tx.send(()).unwrap();

                // block the thread so client can not reply to server keep alive.
                // server would actively drop connection after keepalive timer expired.
                std::thread::sleep(Duration::from_secs(1000));
                drop(c);
                Ok::<_, Error>(())
            })
    });

    rx.recv().unwrap();

    handle.try_handle()?.stop(true);

    let now = Instant::now();

    handle.await?;

    // Default xitca-server shutdown timeout is 30 seconds.
    // If keep-alive functions correctly server shutdown should happen much faster than it.
    assert!(now.elapsed() < Duration::from_secs(30));

    Ok(())
}

async fn handle(req: Request<RequestExt<h2::RequestBody>>) -> Result<Response<ResponseBody>, Error> {
    // Some yield for testing h2 dispatcher's concurrent future handling.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => Ok(Response::new(Bytes::from("GET Response").into())),
        (&Method::GET, "/host") => Ok(Response::new(
            Bytes::from(
                req.headers()
                    .get(header::HOST)
                    .map(|v| v.to_str().unwrap().to_string())
                    .unwrap_or_default(),
            )
            .into(),
        )),
        (&Method::CONNECT, "/") => {
            let (_, mut body) = req.into_parts();
            Ok(Response::new(ResponseBody::box_stream(async_stream::stream! {
                while let Some(chunk) = body.next().await {
                    yield chunk;
                }
            })))
        }
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
