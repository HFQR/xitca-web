use futures_util::{SinkExt, StreamExt};
use http_ws::{Message, ws};
use xitca_client::Client;
use xitca_http::{
    Request,
    body::{Body, BoxBody, StreamDataBody},
    http::{Protocol, RequestExt, Response, Version},
};
use xitca_io::bytes::Bytes;
use xitca_service::fn_service;
use xitca_test::{Error, test_h2_server};

#[tokio::test]
async fn message() -> Result<(), Error> {
    let mut handle = xitca_test::test_h1_server(fn_service(handler))?;

    let c = Client::new();

    let ws = c.ws(&format!("ws://{}", handle.ip_port_string())).send().await?;

    let (mut tx, mut rx) = ws.split();

    for _ in 0..9 {
        tx.send(Message::Text(Bytes::from("Hello,World!"))).await?;
    }

    for _ in 0..9 {
        let msg = rx.next().await.unwrap()?;
        assert_eq!(msg, Message::Text(Bytes::from("Hello,World!")));
    }

    tx.send(Message::Ping(Bytes::from("pingpong"))).await?;
    let msg = rx.next().await.unwrap()?;
    assert_eq!(msg, Message::Pong(Bytes::from("pingpong")));

    tx.send(Message::Close(None)).await?;
    let msg = rx.next().await.unwrap()?;
    assert_eq!(msg, Message::Close(None));

    handle.try_handle()?.stop(true);
    handle.await.map_err(Into::into)
}

#[tokio::test]
async fn message_h2() -> Result<(), Error> {
    let mut handle = test_h2_server(fn_service(handler))?;

    let server_url = format!("wss://{}/", handle.ip_port_string());

    {
        let c = Client::new();
        let (mut tx, mut rx) = c.ws2(&server_url).send().await?.split();

        for _ in 0..9 {
            tx.send(Message::Text(Bytes::from("Hello,World!"))).await?;
        }

        for _ in 0..9 {
            let msg = rx.next().await.unwrap()?;
            assert_eq!(msg, Message::Text(Bytes::from("Hello,World!")));
        }

        tx.send(Message::Ping(Bytes::from("pingpong"))).await?;
        let msg = rx.next().await.unwrap()?;
        assert_eq!(msg, Message::Pong(Bytes::from("pingpong")));

        tx.send(Message::Close(None)).await?;
        let msg = rx.next().await.unwrap()?;
        assert_eq!(msg, Message::Close(None));
    }

    handle.try_handle()?.stop(true);
    tokio::task::yield_now().await;
    handle.await.map_err(Into::into)
}

async fn handler<B>(req: Request<RequestExt<B>>) -> Result<Response<BoxBody>, Error>
where
    B: Body<Data = Bytes> + Unpin + 'static,
    B::Error: 'static,
{
    let (parts, body) = req.into_parts();
    let req = Request::from_parts(parts, ());
    let (ext, body) = body.replace_body(());

    if req.version() == Version::HTTP_2 {
        assert_eq!(ext.protocol().unwrap(), Protocol::WEB_SOCKET);
    }

    let (mut decode, res, mut tx) = ws(&req, StreamDataBody::new(body))?;

    // spawn websocket message handling logic task.
    tokio::task::spawn_local(async move {
        while let Some(Ok(msg)) = decode.next().await {
            match msg {
                Message::Text(bytes) => {
                    tx.text(bytes).await.unwrap();
                }
                Message::Ping(bytes) => {
                    tx.pong(bytes).await.unwrap();
                }
                Message::Close(reason) => {
                    tx.close(reason).await.unwrap();
                    return;
                }
                _ => {}
            }
        }
    });

    Ok(res.map(|stream| BoxBody::new(StreamDataBody::new(stream))))
}
