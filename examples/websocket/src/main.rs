//! A Http/1 server echos back websocket text message.

use std::time::Duration;

use tracing::{error, info};
use xitca_web::{
    handler::{
        handler_service,
        websocket::{Context, Message, WebSocket},
    },
    route::get,
    App,
};

fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("xitca=info,websocket=info")
        .init();
    App::new()
        .at("/", get(handler_service(handler)))
        .serve()
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
        .on_msg(on_msg)
        // async function that called when error occurred
        .on_err(|e| async move { error!("{e}") })
        // async function that called when closing websocket connection.
        .on_close(|| async { info!("WebSocket connection closing") });

    // return the instance after configuration.
    ws
}

// async function that called on every incoming websocket message.
async fn on_msg(msg: Message, ctx: &mut Context) {
    // ignore non text message.
    if let Message::Text(txt) = msg {
        info!("Got text message: {txt}");
        // echo back text.
        ctx.text(format!("Echo: {txt}")).await.unwrap();
    }
}
