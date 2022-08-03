//! A Http/1 server echos back websocket text message.

use std::time::Duration;

use tracing::info;
use xitca_web::{
    handler::handler_service,
    handler::websocket::{Message, WebSocket},
    route::get,
    App, HttpServer,
};

fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("xitca=trace,[xitca-logger]=trace,websocket=info")
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
                    // echo back text message and ignore all other types of message.
                    Message::Text(bytes) => {
                        let str = String::from_utf8_lossy(bytes.as_ref());
                        info!("Got text message {str}");
                        tx.send(Message::Text(format!("Echo: {str}").into())).await.unwrap();
                    }
                    _ => {}
                }
            })
        });

    // return the instance after configuration.
    ws
}
