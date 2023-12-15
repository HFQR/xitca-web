//! static file serving
//! example require nightly Rust to compile
//! example assume you run the application from the path of /examples/file/

use xitca_web::{middleware::compress::Compress, service::file::ServeDir, App};

fn main() -> std::io::Result<()> {
    println!("open http://localhost:8080/index.html in browser to visit the site");
    App::new()
        .at("/", ServeDir::new("static"))
        .enclosed(Compress)
        .serve()
        .bind("localhost:8080")?
        .run()
        .wait()
}
