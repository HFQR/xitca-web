use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tokio::sync::oneshot;
use xitca_client::Client;
use xitca_http::{
    body::{Body, BodyExt, ResponseBody, SizeHint},
    bytes::{Bytes, BytesMut},
    h2,
    http::{HeaderMap, Method, Request, RequestExt, Response, Version, header, uri::Uri},
};
use xitca_service::fn_service;
use xitca_test::{Error, test_h2_server};

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

    handle.try_handle()?.stop(false);

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
        let mut res = c
            .post(&server_url)
            .version(Version::HTTP_2)
            .text(body)
            .trailers(HeaderMap::new())
            .send()
            .await?;
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

/// Two concurrent streams share one h2 connection:
///
/// * `/delayed` — server handler blocks on `RequestBody::poll_frame` because
///   the client never ends its request body.
/// * `/normal` — completes cleanly.
///
/// After `/normal` finishes, the client aborts its connection task so the
/// TCP half-close reaches the server. The dispatcher's read task observes
/// EOF and breaks to `ShutDown::ReadClosed(Ok(()))`, and `try_set_reset`
/// applies `StreamError::GoAway` to every live stream — this is the shared
/// shutdown-drain path. That wakes the delayed handler's `poll_frame` with
/// a `ConnectionAborted` "connection is going away" error; the handler
/// returns any response and exits, letting the connection task drop its
/// service Rc.
///
/// With no in-flight work left, a subsequent graceful `stop(true)` must
/// return quickly — well under the default 30 s shutdown timeout.
#[tokio::test]
async fn h2_shutdown_drains_delayed_stream_on_client_eof() -> Result<(), Error> {
    let (tx_observed, rx_observed) = oneshot::channel::<Option<String>>();
    let tx_observed = Arc::new(Mutex::new(Some(tx_observed)));

    let svc = {
        let tx_observed = tx_observed.clone();
        fn_service(move |req: Request<RequestExt<h2::RequestBody>>| {
            let tx_observed = tx_observed.clone();
            async move {
                match req.uri().path() {
                    "/normal" => {
                        Ok::<Response<ResponseBody>, Error>(Response::new(Bytes::from_static(b"normal").into()))
                    }
                    "/delayed" => {
                        let (_, mut body) = req.into_parts();
                        let mut err_msg = None;
                        while let Some(chunk) = body.frame().await {
                            if let Err(e) = chunk {
                                err_msg = Some(e.to_string());
                                break;
                            }
                        }
                        if let Some(tx) = tx_observed.lock().unwrap().take() {
                            let _ = tx.send(err_msg);
                        }
                        Ok(Response::new(Bytes::from_static(b"delayed-done").into()))
                    }
                    p => Err(format!("unexpected path {p}").into()),
                }
            }
        })
    };

    let mut server = test_h2_server(svc)?;
    let addr = server.addr();

    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let (send, conn) = ::h2::client::handshake(tcp).await?;
    let conn_task = tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut send = send.ready().await?;

    let uri = |path: &str| -> Uri { format!("http://{addr}{path}").parse().unwrap() };

    // Stream A: /delayed — POST with partial body, no END_STREAM. The handler
    // stays parked inside body.frame() until the dispatcher signals shutdown.
    let req_delayed = Request::builder()
        .method(Method::POST)
        .uri(uri("/delayed"))
        .version(Version::HTTP_2)
        .body(())
        .unwrap();
    let (_resp_delayed, mut stream_delayed) = send.send_request(req_delayed, false)?;
    stream_delayed.send_data(Bytes::from_static(b"partial"), false)?;

    // Stream B: /normal — completes cleanly.
    let req_normal = Request::builder()
        .method(Method::GET)
        .uri(uri("/normal"))
        .version(Version::HTTP_2)
        .body(())
        .unwrap();
    let (resp_normal, _) = send.send_request(req_normal, true)?;
    let resp = resp_normal.await?;
    assert_eq!(resp.status().as_u16(), 200);

    // Force the server into its shutdown-drain path: abort the client-side
    // connection task so the TCP half-close reaches the server. The
    // dispatcher's read task observes EOF and breaks to ShutDown::ReadClosed.
    // With io_res == Ok(()), try_set_reset applies StreamError::GoAway to
    // every live stream — this is the forceful-goaway branch.
    conn_task.abort();
    drop(stream_delayed);
    drop(send);
    let _ = conn_task.await;

    // The delayed handler's body.frame() must have woken with the goaway
    // error and reported it back through the oneshot.
    let observed = tokio::time::timeout(Duration::from_secs(5), rx_observed).await??;
    let observed = observed.expect("delayed handler did not observe an error on body.frame()");
    assert!(
        observed.contains("going away"),
        "unexpected body.frame() error: {observed}"
    );

    // Graceful shutdown should return quickly — the connection has already
    // torn down, so nothing keeps the service Rc alive.
    server.try_handle()?.stop(true);
    let now = Instant::now();
    server.await?;
    assert!(
        now.elapsed() < Duration::from_secs(5),
        "graceful shutdown took too long: {:?}",
        now.elapsed()
    );

    Ok(())
}

/// Rapid-reset attack (CVE-2023-44487): the client opens 21 streams and
/// immediately `RST_STREAM`s each one, exceeding the dispatcher's
/// `RESET_MAX` (20) within the sliding window. The decoder's
/// `try_tick_reset` returns `Err(Error::GoAway(ENHANCE_YOUR_CALM))`, which
/// `go_away()` promotes to a fatal queued GOAWAY and breaks to
/// `ShutDown::ReadClosed`.
///
/// * actually emit the GOAWAY frame to the peer with reason
///   `ENHANCE_YOUR_CALM` — asserted on the client side.
/// * drop the service Rc promptly so the subsequent graceful `stop(true)`
///   returns fast.
#[tokio::test]
async fn h2_forceful_goaway_on_rapid_reset_drains_delayed_stream() -> Result<(), Error> {
    let svc = {
        fn_service(move |req: Request<RequestExt<h2::RequestBody>>| async move {
            match req.uri().path() {
                "/normal" => Ok::<Response<ResponseBody>, Error>(Response::new(Bytes::from_static(b"normal").into())),
                p => Err(format!("unexpected path {p}").into()),
            }
        })
    };

    let mut server = test_h2_server(svc)?;
    let addr = server.addr();

    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let (send, conn) = ::h2::client::handshake(tcp).await?;
    // Keep the connection future running so we can observe GOAWAY from the
    // server. Its Err result carries the reason the peer advertised.
    let conn_task = tokio::spawn(conn);
    let mut send = send.ready().await?;

    let uri = |path: &str| -> Uri { format!("http://{addr}{path}").parse().unwrap() };

    // Fire 21 open+reset pairs. Server-side RESET_MAX is 20, so the 21st
    // RST_STREAM trips try_tick_reset → GoAway(ENHANCE_YOUR_CALM).
    for _ in 0..21 {
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri("/normal"))
            .version(Version::HTTP_2)
            .body(())
            .unwrap();
        let (_resp, mut stream) = send.send_request(req, false)?;
        tokio::task::yield_now().await;
        stream.send_reset(::h2::Reason::CANCEL);
    }

    // Wait for the server's GOAWAY to reach us. The client's Connection
    // future completes with an error carrying the remote-advertised reason.
    let conn_res = conn_task.await?;
    let err = conn_res.expect_err("expected GOAWAY error from server, got Ok");
    assert!(err.is_go_away(), "expected GOAWAY, got {err:?}");
    assert!(err.is_remote(), "GOAWAY should be peer-initiated, got {err:?}");
    assert_eq!(
        err.reason(),
        Some(::h2::Reason::ENHANCE_YOUR_CALM),
        "unexpected GOAWAY reason"
    );

    // Graceful shutdown should complete quickly — the connection has already
    // torn down, so nothing keeps the service Rc alive.
    server.try_handle()?.stop(true);
    let now = Instant::now();
    server.await?;
    assert!(
        now.elapsed() < Duration::from_secs(5),
        "graceful shutdown took too long: {:?}",
        now.elapsed()
    );

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
            Ok(Response::new(ResponseBody::boxed(xitca_http::body::StreamBody::new(
                async_stream::stream! {
                    while let Some(chunk) = body.frame().await {
                        yield chunk;
                    }
                },
            ))))
        }
        (&Method::POST, "/") => {
            let (parts, mut body) = req.into_parts();

            let length = parts.headers.get(header::CONTENT_LENGTH).unwrap().to_str()?.parse()?;

            let size = body.size_hint();

            assert_eq!(size, SizeHint::Exact(length));

            let mut buf = BytesMut::new();

            while let Some(bytes) = body.data().await {
                buf.extend_from_slice(&bytes?);
            }

            assert_eq!(buf.len(), length as usize);

            Ok(Response::new(Bytes::new().into()))
        }
        _ => todo!(),
    }
}
