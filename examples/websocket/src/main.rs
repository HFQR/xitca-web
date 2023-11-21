//! A Http/1 server echos back websocket text message.

use xitca_web::{
    handler::{
        handler_service,
        websocket::{Context, Message, WebSocket},
    },
    route::get,
    App,
};

fn main() -> std::io::Result<()> {
    App::new()
        .at("/", get(handler_service(handler)))
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

async fn handler(mut ws: WebSocket) -> WebSocket {
    // config websocket.
    ws.on_msg(on_msg);
    // return the instance after configuration.
    ws
}

// async function that called on every incoming websocket message.
async fn on_msg(msg: Message, ctx: &mut Context) {
    // ignore non text message.
    if let Message::Text(txt) = msg {
        println!("Got text message: {txt}");
        // echo back text.
        ctx.text(format!("Echo: {txt}")).await.unwrap();
    }
}
