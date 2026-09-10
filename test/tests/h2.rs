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
    http::{HeaderMap, Method, Request, RequestExt, Response, StatusCode, Version, header, uri::Uri},
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

/// Two uploads consume the entire 65,535-byte connection window while each
/// remains below the per-stream receive threshold (49,149 bytes). Connection
/// credit must be returned even though neither stream has reached its threshold.
#[tokio::test]
async fn h2_concurrent_uploads_replenish_connection_window() -> Result<(), Error> {
    const UPLOAD_SIZE: usize = 64 * 1024;
    const PREFIX_SIZES: [usize; 2] = [32_768, 32_767];

    let (tx_consumed, mut rx_consumed) = tokio::sync::mpsc::unbounded_channel();
    let svc = fn_service(move |req: Request<RequestExt<h2::RequestBody>>| {
        let tx_consumed = tx_consumed.clone();
        async move {
            let index = match req.uri().path() {
                "/upload-a" => 0,
                "/upload-b" => 1,
                path => return Err(format!("unexpected path {path}").into()),
            };
            let mut body = req.into_body();
            let mut received = 0;
            while let Some(data) = body.data().await {
                received += data?.len();
                if received == PREFIX_SIZES[index] {
                    // Observe consumption without suspending the handler. It
                    // continues reading and naturally waits in Body::poll_frame.
                    let _ = tx_consumed.send((index, received));
                }
            }

            Ok::<Response<ResponseBody>, Error>(
                Response::builder()
                    .header("x-uploaded-bytes", received.to_string())
                    .body(Bytes::new().into())?,
            )
        }
    });

    let mut server = test_h2_server(svc)?;
    let addr = server.addr();
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let (send, conn) = ::h2::client::handshake(tcp).await?;
    let conn_task = tokio::spawn(conn);

    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut send = send.ready().await?;
        let request = |path| {
            Request::builder()
                .method(Method::POST)
                .uri(format!("http://{addr}{path}"))
                .version(Version::HTTP_2)
                .header(header::CONTENT_LENGTH, UPLOAD_SIZE)
                .body(())
        };
        let (response_a, mut stream_a) = send.send_request(request("/upload-a")?, false)?;
        let (response_b, mut stream_b) = send.send_request(request("/upload-b")?, false)?;
        let mut data_a = Bytes::from(vec![b'a'; UPLOAD_SIZE]);
        let mut data_b = Bytes::from(vec![b'b'; UPLOAD_SIZE]);

        // h2 splits these into legal DATA frames. Together they use exactly the
        // initial connection window, leaving both request bodies unfinished.
        stream_a.send_data(data_a.split_to(PREFIX_SIZES[0]), false)?;
        stream_b.send_data(data_b.split_to(PREFIX_SIZES[1]), false)?;

        let mut consumed = [0; 2];
        while consumed != PREFIX_SIZES {
            let (index, bytes) = rx_consumed
                .recv()
                .await
                .ok_or("upload handlers stopped before consuming prefixes")?;
            consumed[index] = bytes;
        }

        // More upload data is now available. It cannot reach the server until
        // connection credit is replenished; stream credit alone is sufficient.
        stream_a.send_data(data_a, true)?;
        stream_b.send_data(data_b, true)?;
        tokio::time::timeout(
            Duration::from_secs(2),
            futures_util::future::try_join(response_a, response_b),
        )
        .await
        .map_err(|_| {
            format!(
                "uploads stalled after the server consumed {consumed:?} bytes: \
                 the exhausted connection window must be replenished below the per-stream threshold"
            )
        })?
        .map_err(Error::from)
    })
    .await;

    // Clean up even when the regression times out, before reporting failure.
    conn_task.abort();
    let _ = conn_task.await;
    server.try_handle()?.stop(false);
    server.await?;

    let (response_a, response_b) = result??;
    for response in [response_a, response_b] {
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-uploaded-bytes"], UPLOAD_SIZE.to_string());
    }
    Ok(())
}

#[tokio::test]
async fn h2_body_consumed_in_separate_task_wakes_writer() -> Result<(), Error> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use xitca_http::{HttpServiceBuilder, config::HttpServiceConfig};
    use xitca_service::ServiceExt;

    const UPLOAD_SIZE: usize = 256 * 1024;

    let consumed = Arc::new(AtomicUsize::new(0));
    let svc = {
        let consumed = consumed.clone();
        fn_service(move |req: Request<RequestExt<h2::RequestBody>>| {
            let consumed = consumed.clone();
            async move {
                // The dispatcher polls this JoinHandle, while a separate task
                // polls RequestBody. The handle wakes only when reading ends.
                let received = tokio::task::spawn_local(async move {
                    let mut body = req.into_body();
                    let mut received = 0;
                    while let Some(data) = body.data().await {
                        received += data?.len();
                        consumed.store(received, Ordering::Relaxed);
                    }
                    Ok::<_, Error>(received)
                })
                .await??;

                Ok::<Response<ResponseBody>, Error>(
                    Response::builder()
                        .header("x-uploaded-bytes", received.to_string())
                        .body(Bytes::new().into())?,
                )
            }
        })
    };

    // The normal test helper uses a 500 ms keepalive. A timer waking the
    // dispatcher could flush queued credit and conceal the missing body wake.
    let builder = HttpServiceBuilder::h2().config(HttpServiceConfig::new().keep_alive_timeout(Duration::from_secs(30)));
    #[cfg(feature = "io-uring")]
    let builder = builder.io_uring();
    let mut server = xitca_test::test_server(svc.enclosed(builder))?;
    let tcp = tokio::net::TcpStream::connect(server.addr()).await?;
    let (send, conn) = ::h2::client::handshake(tcp).await?;
    let conn_task = tokio::spawn(conn);

    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let mut send = send.ready().await?;
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("http://{}/upload", server.addr()))
            .version(Version::HTTP_2)
            .header(header::CONTENT_LENGTH, UPLOAD_SIZE)
            .body(())?;
        let (response, mut stream) = send.send_request(request, false)?;

        // Several windows of DATA require repeated credit returns after the
        // handshake traffic ends. No response is produced until body EOF.
        stream.send_data(Bytes::from(vec![b'x'; UPLOAD_SIZE]), true)?;
        response.await.map_err(Error::from)
    })
    .await;
    let received = consumed.load(Ordering::Relaxed);

    // Clean up before reporting a timeout, including the stalled reader task.
    conn_task.abort();
    let _ = conn_task.await;
    server.try_handle()?.stop(false);
    server.await?;

    let response = result.map_err(|_| {
        format!(
            "separate body task consumed {received} of {UPLOAD_SIZE} bytes, but the upload stalled: \
             queued WINDOW_UPDATE frames must wake the dispatcher"
        )
    })??;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-uploaded-bytes"], UPLOAD_SIZE.to_string());
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
/// A `/delayed` stream is opened first and left parked inside
/// `RequestBody::poll_frame`. Once the forceful GOAWAY fires, the shutdown
/// drain must:
///
/// * actually emit the GOAWAY frame to the peer with reason
///   `ENHANCE_YOUR_CALM` — asserted on the client side.
/// * apply `StreamError::GoAway` to the delayed stream so its `poll_frame`
///   wakes with "connection is going away" — asserted via a oneshot from
///   the handler.
/// * drop the service Rc promptly so the subsequent graceful `stop(true)`
///   returns fast.
#[tokio::test]
async fn h2_forceful_goaway_on_rapid_reset_drains_delayed_stream() -> Result<(), Error> {
    let svc = fn_service(move |req: Request<RequestExt<h2::RequestBody>>| async move {
        match req.uri().path() {
            "/normal" => Ok::<Response<ResponseBody>, Error>(Response::new(Bytes::from_static(b"normal").into())),
            "/delayed" => {
                let (_, mut body) = req.into_parts();
                while let Some(chunk) = body.frame().await {
                    if chunk.is_err() {
                        return Ok(Response::builder()
                            .status(StatusCode::IM_A_TEAPOT)
                            .body(Bytes::from_static(b"delayed-done").into())
                            .unwrap());
                    }
                }
                unreachable!("delayed stream should be going away");
            }
            p => Err(format!("unexpected path {p}").into()),
        }
    });

    let mut server = test_h2_server(svc)?;
    let addr = server.addr();

    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let (send, conn) = ::h2::client::handshake(tcp).await?;
    // Keep the connection future running so we can observe GOAWAY from the
    // server. Its Err result carries the reason the peer advertised.
    let conn_task = tokio::spawn(conn);
    let mut send = send.ready().await?;

    let uri = |path: &str| -> Uri { format!("http://{addr}{path}").parse().unwrap() };
    {
        // Park a /delayed stream inside body.frame() on the server.
        let req_delayed = Request::builder()
            .method(Method::POST)
            .uri(uri("/delayed"))
            .version(Version::HTTP_2)
            .body(())
            .unwrap();
        let (resp_delayed, mut stream_delayed) = send.send_request(req_delayed, false)?;
        stream_delayed.send_data(Bytes::from_static(b"partial"), false)?;

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

        let res = resp_delayed.await?.status();
        assert_eq!(res, StatusCode::IM_A_TEAPOT);
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
        (&Method::POST, "/empty") => {
            let (parts, mut body) = req.into_parts();

            assert_eq!(parts.headers.get(header::CONTENT_LENGTH).unwrap(), "0");
            assert!(body.data().await.is_none());

            Ok(Response::new(Bytes::new().into()))
        }
        _ => todo!(),
    }
}

#[tokio::test]
async fn h2_post_empty_body() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;

    let server_url = format!("https://{}/empty", handle.ip_port_string());

    let c = Client::new();

    let res = tokio::time::timeout(
        Duration::from_secs(5),
        c.post(&server_url).version(Version::HTTP_2).text("").send(),
    )
    .await??;

    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(res.string().await?, "");

    handle.try_handle()?.stop(false);
    handle.await?;

    Ok(())
}
