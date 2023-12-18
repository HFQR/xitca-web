//! use tower_http ServeDir service and SetResponseHeaderLayer layer with xitca-web to serve file.
//!
//! *. to simplify runtime path detection this example assume you always run it from path
//! examples/tower-http/

use tower_http::{compression::CompressionLayer, services::ServeDir};
use xitca_web::{
    http::Uri, middleware::tower_http_compat::TowerHttpCompat as CompatMiddleware,
    service::tower_http_compat::TowerHttpCompat as CompatBuild, service::Service, App, WebContext,
};

fn main() -> std::io::Result<()> {
    App::new()
        .at(
            // catch all request path and pass it to ServeDir service where the path is matched against file.
            "/*path",
            // use CompatBuild to wrap tower-http service which would transform it to a xitca service.
            CompatBuild::new(ServeDir::new("files")),
        )
        // use CompatMiddleware to wrap tower-http middleware layer which would transform it to a xitca middleware.
        .enclosed(CompatMiddleware::new(CompressionLayer::new()))
        // a simple middleware to intercept empty path and replace it with index.html
        .enclosed_fn(path)
        .serve()
        .bind("localhost:8080")?
        .run()
        .wait()
}

async fn path<Res, Err>(
    service: &impl for<'r> Service<WebContext<'r>, Response = Res, Error = Err>,
    mut ctx: WebContext<'_>,
) -> Result<Res, Err> {
    match ctx.req().uri().path() {
        "/" | "" => *ctx.req_mut().uri_mut() = Uri::from_static("/index.html"),
        _ => {}
    }
    service.call(ctx).await
}
