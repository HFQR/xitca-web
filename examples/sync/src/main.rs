//! sync version of hello-world example in this repo.

use xitca_web::{handler::handler_sync_service, route::get, App, HttpServer};

fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .at("/", get(handler_sync_service(|| "Hello,World!")))
            .finish()
    })
    .bind("127.0.0.1:8080")?
    .run()
    .wait()
}
