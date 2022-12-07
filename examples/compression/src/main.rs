//! A Http server decompress request body and compress response body.

use std::io;

use tracing::info;
use xitca_web::{
    handler::handler_service,
    middleware::{compress::Compress, decompress::Decompress},
    App, HttpServer,
};

fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("xitca=trace,[xitca-logger]=trace")
        .init();
    HttpServer::new(|| {
        App::new()
            .at("/", handler_service(root))
            .enclosed(Compress)
            .enclosed(Decompress)
            .finish()
    })
    .bind("127.0.0.1:8080")?
    .run()
    .wait()
}

async fn root(string: String) -> &'static str {
    info!("Request body is: {}", string);
    "Hello World!"
}
