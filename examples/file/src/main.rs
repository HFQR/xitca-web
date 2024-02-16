//! static file serving example.
//! require nightly Rust to compile
//! example assume you run the application from the path of /examples/file/

use xitca_web::{
    handler::redirect::Redirect,
    middleware::{compress::Compress, Logger},
    service::file::ServeDir,
    App,
};

fn main() -> std::io::Result<()> {
    let app = App::new()
        // redirect user visiting "/" path to index file.
        // this service has higher priority than following serve dir service.
        .at("/", Redirect::see_other("/index.html"))
        /*
            map "/" prefixed uri to "./static" file path. for example http request with uri of
            "/index.html" will be matched against "./static/index.html" file.
        */
        .at("/", ServeDir::new("static"))
        // compression middleware
        .enclosed(Compress)
        // logger middleware
        .enclosed(Logger::new())
        .serve()
        .bind("localhost:8080")?
        .run();

    tracing::info!("open http://localhost:8080/ in browser to visit the site");

    app.wait()
}
