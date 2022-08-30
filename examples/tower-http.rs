//! use tower_http ServeDir service with xitca-web to serve file.
//!
//! *. to simplify runtime path detection this example assume you always run it from /examples path.

use tower_http::services::ServeDir;
use xitca_web::{
    dev::service::Service, http::Uri, request::WebRequest, service::tower_http_compat::TowerHttpCompat as CompatBuild,
    App, HttpServer,
};

fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .at(
                // catch all request path and pass it to ServeDir service where the path is matched against file.
                "/*path",
                // use CompatBuild to wrap any tower-http service which would transfomr it to a xitca service.
                CompatBuild::new(ServeDir::new("files")),
            )
            .enclosed_fn(index)
            .finish()
    })
    .bind("localhost:8080")?
    .run()
    .wait()
}

// a simple middleware to intercept empty path and replace it with index.html
async fn index<Res, Err>(
    service: &impl for<'r> Service<WebRequest<'r>, Response = Res, Error = Err>,
    mut req: WebRequest<'_>,
) -> Result<Res, Err> {
    match req.req().uri().path() {
        "/" | "" => *req.req_mut().uri_mut() = Uri::from_static("/index.html"),
        _ => {}
    }
    service.call(req).await
}
