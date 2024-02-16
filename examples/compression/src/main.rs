//! A Http server decompress request body and compress response body.

use std::io;

use tracing::info;
use xitca_web::{
    handler::handler_service,
    middleware::{compress::Compress, decompress::Decompress, Logger},
    App,
};

fn main() -> io::Result<()> {
    App::new()
        .at("/", handler_service(root))
        .enclosed(Compress)
        .enclosed(Decompress)
        .enclosed(Logger::new())
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

async fn root(string: String) -> &'static str {
    info!("Request body is: {}", string);
    "Hello World!"
}
