//! A Http/1 server echos back websocket text message.

use std::time::Duration;

use tracing::{error, info};
use xitca_web::{
    handler::handler_service,
    handler::websocket::{Message, WebSocket},
    route::get,
    App, HttpServer,
};

fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("xitca=info,websocket=info")
        .init();
    HttpServer::new(|| App::new().at("/", get(handler_service(handler))).finish())
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

async fn handler(mut ws: WebSocket) -> WebSocket {
    // config websocket.
    ws
        // interval of ping message sent to client.
        .set_ping_interval(Duration::from_secs(5))
        // max consecutive unanswered pings until server close connection
        .set_max_unanswered_ping(2)
        // async function that called on every incoming websocket message.
        .on_msg(|tx, msg| {
            Box::pin(async move {
                match msg {
                    // echo back text message.
                    Message::Text(bytes) => {
                        let str = String::from_utf8_lossy(bytes.as_ref());
                        info!("Got text message: {str}");
                        tx.text(format!("Echo: {str}")).await.unwrap();
                    }
                    Message::Ping(_) | Message::Pong(_) | Message::Close(_) => {
                        unreachable!("ping/pong/close messages are managed automatically by WebSocket type and exclueded from on_msg function");
                    }
                    _ => {
                        // ignore all other types of message.
                    }
                }
            })
        })
        // async function that called when error occurred
        .on_err(|e| Box::pin(async move { error!("{e}") }))
        // async function that called when closing websocket connection.
        .on_close(|| Box::pin(async { info!("WebSocket connection closing") }));

    // return the instance after configuration.
    ws
}
