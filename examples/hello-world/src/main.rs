//! A Http server returns Hello World String as Response.

use xitca_web::{handler::handler_service, route::get, App, HttpServer};

fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .at("/", get(handler_service(|| async { "Hello,World!" })))
            .finish()
    })
    .bind("127.0.0.1:8080")?
    .run()
    .wait()
}
