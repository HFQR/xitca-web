//! use tower_http ServeDir service and SetResponseHeaderLayer layer with xitca-web to serve file.
//!
//! *. to simplify runtime path detection this example assume you always run it from path
//! examples/tower-http/

use tower_http::{compression::CompressionLayer, services::ServeDir};
use xitca_web::{
    handler::redirect::Redirect,
    middleware::{eraser::TypeEraser, tower_http_compat::TowerHttpCompat as CompatMiddleware},
    service::{tower_http_compat::TowerHttpCompat as CompatBuild, ServiceExt},
    App,
};

fn main() -> std::io::Result<()> {
    App::new()
        .at("/", Redirect::see_other("/index.html"))
        .at(
            // catch all request path and pass it to ServeDir service where the path is matched against file.
            "/*path",
            // use CompatBuild to wrap tower-http service which would transform it to a xitca service.
            CompatBuild::new(ServeDir::new("files"))
                // middleware for erasing complex types created by tower_http.
                .enclosed(TypeEraser::error())
                .enclosed(TypeEraser::response_body()),
        )
        // use CompatMiddleware to wrap tower-http middleware layer which would transform it to a xitca middleware.
        .enclosed(CompatMiddleware::new(CompressionLayer::new()))
        .serve()
        .bind("localhost:8080")?
        .run()
        .wait()
}
