//! Tests for xitca-http's h2 server RST_STREAM behavior when the service
//! drops the request body early. Uses the h2 crate directly so the client
//! can observe both response frames AND RST_STREAM frames (xitca-client
//! drains request body first and errors out on reset).

use core::{future::poll_fn, time::Duration};
use std::net::SocketAddr;

use h2::{Reason, client::SendRequest};
use tokio::{net::TcpStream, task::JoinHandle};
use xitca_http::{
    body::{Frame, ResponseBody, StreamBody},
    bytes::Bytes,
    error::BodyError,
    h2::RequestBody,
    http::{Method, Request, RequestExt, Response, Version, uri::Uri},
};
use xitca_service::fn_service;
use xitca_test::{Error, test_h2_server};

async fn connect(addr: SocketAddr) -> Result<(SendRequest<Bytes>, JoinHandle<()>), Error> {
    let tcp = TcpStream::connect(addr).await?;
    let (send, conn) = h2::client::handshake(tcp).await?;
    let task = tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok((send, task))
}

fn uri(addr: SocketAddr, path: &str) -> Uri {
    format!("http://{addr}{path}").parse().unwrap()
}

async fn handle(req: Request<RequestExt<RequestBody>>) -> Result<Response<ResponseBody>, Error> {
    // Yield so dispatcher has a chance to process any buffered DATA frames
    // before the handler mutates stream state via body drop.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    match req.uri().path() {
        "/drop" => {
            drop(req.into_body());
            Ok(Response::new(Bytes::new().into()))
        }
        "/drop_sleep" => {
            drop(req.into_body());
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(Response::new(Bytes::new().into()))
        }
        "/drop_err" => {
            drop(req.into_body());
            Err("service error after body drop".into())
        }
        "/drop_resp_body" => {
            drop(req.into_body());
            let body = StreamBody::new(async_stream::stream! {
                for i in 0..4u32 {
                    yield Ok::<_, BodyError>(Frame::Data(Bytes::from(format!("chunk-{i}"))));
                }
            });
            Ok(Response::new(ResponseBody::boxed(body)))
        }
        path => Err(format!("unexpected path {path}").into()),
    }
}

// 1MB body. With default 64KB initial stream window, the client's send side
// blocks on capacity long before the body is fully transmitted — guaranteeing
// recv.state is still `Open` at the server when the handler drops the body.
const LARGE_BODY: usize = 1024 * 1024;

fn large_body() -> Bytes {
    Bytes::from(vec![b'x'; LARGE_BODY])
}

/// Scenario 1: client streams a large body that overruns its send window,
/// handler drops the body, handler returns headers-only response.
/// Client observes HEADERS(end_stream) followed by RST_STREAM(NO_ERROR).
/// Exercises the `(State::Cancel, State::Close)` → `TryRemove::ResetCancel`
/// path and in-flight DATA absorption.
#[tokio::test]
async fn drop_large_body_empty_response() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;
    let addr = handle.addr();

    let (send, _conn_task) = connect(addr).await?;
    let mut send = send.ready().await?;

    let req = Request::builder()
        .method(Method::POST)
        .uri(uri(addr, "/drop"))
        .version(Version::HTTP_2)
        .body(())
        .unwrap();

    let (resp_fut, mut stream) = send.send_request(req, false)?;
    // send_data without end_stream — keeps recv.state == Open at the server
    // so the body drop transitions Open → Cancel and queues NoError.
    stream.send_data(large_body(), false)?;

    let resp = resp_fut.await?;
    assert_eq!(resp.status().as_u16(), 200);

    let reason = poll_fn(|cx| stream.poll_reset(cx)).await?;
    assert_eq!(reason, Reason::NO_ERROR);

    drop(send);
    drop(stream);
    drop(resp);
    handle.try_handle()?.stop(false);
    handle.await?;
    Ok(())
}

/// Scenario 2: client sends a small body in two frames (data, then empty
/// end_stream) with a pause in between. The handler drops the body while
/// recv is still `Open`, then sleeps, then responds. The peer's END_STREAM
/// arrives during the sleep, exercising `promote_cancel_to_close_recv` →
/// `try_revert_cancel_error` so no RST is emitted.
#[tokio::test]
async fn drop_small_body_end_stream_reverts_reset() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;
    let addr = handle.addr();

    let (send, _conn_task) = connect(addr).await?;
    let mut send = send.ready().await?;

    let req = Request::builder()
        .method(Method::POST)
        .uri(uri(addr, "/drop_sleep"))
        .version(Version::HTTP_2)
        .body(())
        .unwrap();

    let (resp_fut, mut stream) = send.send_request(req, false)?;
    stream.send_data(Bytes::from_static(b"hello"), false)?;

    // Give the handler time to yield, drop body, and enter sleep before we
    // deliver END_STREAM. 50ms is well under the 500ms keepalive timeout the
    // test server uses.
    tokio::time::sleep(Duration::from_millis(50)).await;
    stream.send_data(Bytes::new(), true)?;

    let resp = resp_fut.await?;
    assert_eq!(resp.status().as_u16(), 200);

    // Poll for reset with a timeout — we assert NO reset arrives. If the
    // revert path regressed, the server would send RST(NO_ERROR) here.
    let reset = tokio::time::timeout(Duration::from_millis(300), poll_fn(|cx| stream.poll_reset(cx))).await;
    assert!(reset.is_err(), "expected no RST_STREAM, got {reset:?}");

    drop(send);
    drop(stream);
    drop(resp);
    handle.try_handle()?.stop(false);
    handle.await?;
    Ok(())
}

/// Scenario 3: handler drops body, then returns `Err`. The body drop queues
/// `NoError`; the service error upgrades it to `InternalError` via the
/// priority logic in `try_set_pending_error`. Client receives no HEADERS —
/// only RST_STREAM(INTERNAL_ERROR).
#[tokio::test]
async fn drop_body_service_error_upgrades_to_internal() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;
    let addr = handle.addr();

    let (send, _conn_task) = connect(addr).await?;
    let mut send = send.ready().await?;

    let req = Request::builder()
        .method(Method::POST)
        .uri(uri(addr, "/drop_err"))
        .version(Version::HTTP_2)
        .body(())
        .unwrap();

    let (resp_fut, _stream) = send.send_request(req, false)?;

    let err = resp_fut.await.expect_err("expected stream reset, got response");
    assert!(err.is_reset(), "expected reset error, got {err:?}");
    assert_eq!(err.reason(), Some(Reason::INTERNAL_ERROR));

    drop(send);
    handle.try_handle()?.stop(false);
    handle.await?;
    Ok(())
}

/// Scenario 5: response carries a streaming DATA body. All DATA frames must
/// be delivered before the RST_STREAM(NO_ERROR) that the StreamGuard drop
/// queues. The client may observe the terminal RST via either the body
/// stream (as a Reset(NO_ERROR) error after all chunks) or via poll_reset
/// after a clean END_STREAM — both are acceptable orderings.
#[tokio::test]
async fn drop_body_with_response_data_body() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handle))?;
    let addr = handle.addr();

    let (send, _conn_task) = connect(addr).await?;
    let mut send = send.ready().await?;

    let req = Request::builder()
        .method(Method::POST)
        .uri(uri(addr, "/drop_resp_body"))
        .version(Version::HTTP_2)
        .body(())
        .unwrap();

    let (resp_fut, mut stream) = send.send_request(req, false)?;
    stream.send_data(large_body(), false)?;

    let resp = resp_fut.await?;
    assert_eq!(resp.status().as_u16(), 200);

    let mut body = resp.into_body();
    let mut collected = Vec::new();
    let mut terminal_reset = None;
    while let Some(chunk) = body.data().await {
        match chunk {
            Ok(c) => collected.extend_from_slice(&c),
            Err(e) => {
                terminal_reset = Some(e);
                break;
            }
        }
    }
    assert_eq!(collected, b"chunk-0chunk-1chunk-2chunk-3");

    // Either the body ended cleanly (poll_reset yields NO_ERROR) or the body
    // iteration bailed on a Reset(NO_ERROR) frame.
    match terminal_reset {
        Some(e) => assert_eq!(e.reason(), Some(Reason::NO_ERROR), "unexpected reset reason: {e:?}"),
        None => {
            let reason = poll_fn(|cx| stream.poll_reset(cx)).await?;
            assert_eq!(reason, Reason::NO_ERROR);
        }
    }

    drop(send);
    drop(stream);
    drop(body);
    handle.try_handle()?.stop(false);
    handle.await?;
    Ok(())
}

/// Scenario 7: many concurrent streams all drop their bodies. Every stream
/// should receive RST_STREAM(NO_ERROR) and the connection must stay up —
/// server-initiated NoError resets are excluded from the rapid-reset counter,
/// so no ENHANCE_YOUR_CALM GOAWAY is triggered.
#[tokio::test]
async fn many_concurrent_drops_no_goaway() -> Result<(), Error> {
    const STREAMS: usize = 40;

    let mut handle = test_h2_server(fn_service(handle))?;
    let addr = handle.addr();

    let (send, conn_task) = connect(addr).await?;

    let mut join = Vec::with_capacity(STREAMS);
    for _ in 0..STREAMS {
        let send = send.clone();
        join.push(tokio::spawn(async move {
            let mut send = send.ready().await?;
            let req = Request::builder()
                .method(Method::POST)
                .uri(uri(addr, "/drop"))
                .version(Version::HTTP_2)
                .body(())
                .unwrap();
            let (resp_fut, mut stream) = send.send_request(req, false)?;
            stream.send_data(large_body(), false)?;

            let resp = resp_fut.await?;
            assert_eq!(resp.status().as_u16(), 200);

            let reason = poll_fn(|cx| stream.poll_reset(cx)).await?;
            assert_eq!(reason, Reason::NO_ERROR);
            Ok::<(), Error>(())
        }));
    }

    for j in join {
        j.await.unwrap()?;
    }

    // Connection must still be alive. If the server had sent GOAWAY, the
    // connection task would have completed.
    assert!(
        !conn_task.is_finished(),
        "connection dropped — likely GOAWAY(ENHANCE_YOUR_CALM)"
    );

    drop(send);
    handle.try_handle()?.stop(false);
    handle.await?;
    Ok(())
}
