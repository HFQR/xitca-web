use futures_util::{SinkExt, StreamExt, TryStreamExt};
use http_ws::{ws, Message};
use xitca_http::{
    body::ResponseBody,
    error::BodyError,
    h1,
    http::{Request, Response},
};
use xitca_io::bytes::Bytes;
use xitca_service::fn_service;
use xitca_test::Error;

#[tokio::test]
async fn message() -> Result<(), Error> {
    let mut handle = xitca_test::test_h1_server(|| fn_service(handler))?;

    let c = xitca_client::Client::new();

    let ws = c.ws(&format!("ws://{}", handle.ip_port_string()))?.send().await?.ws()?;

    let (mut tx, mut rx) = ws.split();

    tx.send(Message::Text(Bytes::from("Hello,World!"))).await?;
    let msg = rx.next().await.unwrap()?;
    assert_eq!(msg, Message::Text(Bytes::from("Hello,World!")));

    tx.send(Message::Ping(Bytes::from("pingpong"))).await?;
    let msg = rx.next().await.unwrap()?;
    assert_eq!(msg, Message::Pong(Bytes::from("pingpong")));

    tx.send(Message::Close(None)).await?;
    let msg = rx.next().await.unwrap()?;
    assert_eq!(msg, Message::Close(None));

    handle.try_handle()?.stop(true);

    handle.await.map_err(Into::into)
}

async fn handler(req: Request<h1::RequestBody>) -> Result<Response<ResponseBody>, Error> {
    let (mut decode, res, tx) = ws(req)?;

    // spawn websocket message handling logic task.
    tokio::task::spawn_local(async move {
        while let Some(Ok(msg)) = decode.next().await {
            match msg {
                Message::Text(bytes) => {
                    tx.send(Message::Text(bytes)).await.unwrap();
                }
                Message::Ping(bytes) => {
                    tx.send(Message::Pong(bytes)).await.unwrap();
                }
                Message::Close(reason) => {
                    tx.send(Message::Close(reason)).await.unwrap();
                    return;
                }
                _ => {}
            }
        }
    });

    let (parts, body) = res.into_parts();
    let body = body.map_err(|e| BodyError::from(Box::new(e) as Error));
    let body = ResponseBody::stream(Box::pin(body) as _);
    let res = Response::from_parts(parts, body);

    Ok(res)
}
