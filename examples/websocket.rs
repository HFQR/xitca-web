//! A Http/1 server echos back websocket text message and respond to ping message.

use actix_http_alt::{
    http::{Request, Response},
    BodyError, RequestBody, ResponseBody,
};
use actix_service_alt::fn_service;
use actix_web_alt::HttpServer;
use futures_util::TryStreamExt;
use http_ws::{ws, Message};
use log::info;

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    // construct http server
    HttpServer::new(|| fn_service(handler))
        .max_head_size::<{ 1024 * 1024 * 8 }>()
        .bind("127.0.0.1:8080")?
        .run()
        .await
}

async fn handler(req: Request<RequestBody>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>> {
    let (mut decode, res, tx) = ws(req).map_err(|e| format!("{:?}", e))?;

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

    let (parts, body) = res.into_parts();
    let body = body.map_err(|e| BodyError::Std(format!("{:?}", e).into()));
    let body = Box::pin(body) as _;
    let body = ResponseBody::stream(body);
    let res = Response::from_parts(parts, body);

    Ok(res)
}
