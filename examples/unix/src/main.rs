//! A UnixDomain server returns Hello World String as Response.

use xitca_web::{handler::handler_service, route::get, App, HttpServer};

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    #[allow(unused_mut)]
    let mut server = HttpServer::new(|| App::new().at("/", get(handler_service(handler))).finish());

    #[cfg(unix)]
    {
        server = server.bind_unix("/tmp/xitca-web.socket")?;
    }

    #[cfg(not(unix))]
    {
        server = server.bind("127.0.0.1:8080")?;
    }

    server.run().await
}

async fn handler() -> &'static str {
    "Hello,World!"
}
