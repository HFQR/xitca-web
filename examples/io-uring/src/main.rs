//! A Http server returns Hello World String as Response with io-uring runtime

use xitca_web::{App, handler::handler_service, route::get};

fn main() -> std::io::Result<()> {
    App::new()
        .at("/", get(handler_service(async || "Hello,World!")))
        .serve()
        .enable_io_uring()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}
