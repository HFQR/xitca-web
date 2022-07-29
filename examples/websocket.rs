//! A Http/1 server echos back websocket text message and respond to ping message.

use futures_util::TryStreamExt;
use http_ws::{ws, Message};
use tracing::info;
use xitca_web::{
    dev::service::fn_service,
    error::BodyError,
    http,
    request::WebRequest,
    response::{ResponseBody, WebResponse},
    route::get,
    App, HttpServer,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    // use tokio tracing for log.
    tracing_subscriber::fmt()
        .with_env_filter("xitca=trace,[xitca-logger]=trace,websocket=info")
        .init();

    // some state shared in http server.
    let shared_state = "app_state";

    // construct http server
    HttpServer::new(move || {
        // construct an app with state and handler.
        App::with_multi_thread_state(shared_state)
            .at("/", get(fn_service(handler)))
            .finish()
    })
    .max_write_buf_size::<16>()
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

async fn handler(mut req: WebRequest<'_, &'static str>) -> Result<WebResponse, Box<dyn std::error::Error>> {
    // borrow shared state of App.
    let state = req.state();
    assert_eq!(*state, "app_state");

    // take ownership of request.
    let (parts, body) = req.take_request().into_parts();
    let req = http::Request::from_parts(parts, ());

    // construct websocket handler types.
    let (mut decode, res, tx) = ws(req, body)?;

    // spawn websocket message handling logic task.
    tokio::task::spawn_local(async move {
        while let Some(Ok(msg)) = decode.next().await {
            match msg {
                Message::Text(bytes) => {
                    let str = String::from_utf8_lossy(bytes.as_ref());
                    info!("Got text message {:?}", str);
                    tx.send(Message::Text(format!("Echo: {}", str).into())).await.unwrap();
                }
                Message::Ping(bytes) => {
                    info!("Got ping message");
                    tx.send(Message::Pong(bytes)).await.unwrap();
                }
                Message::Close(reason) => {
                    info!("Got close message");
                    tx.send(Message::Close(reason)).await.unwrap();
                    return;
                }
                _ => {}
            }
        }
    });

    // construct response types.
    Ok(res.map(|body| {
        let body = body.map_err(|e| BodyError::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>));
        ResponseBody::stream(Box::pin(body) as _)
    }))
}
