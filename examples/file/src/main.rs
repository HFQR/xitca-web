//! a low level static file serving example where the serve service is shared as application
//! state rather than a service type sit at fixed route endpoint.

use http_file::ServeDir;
use xitca_web::{
    body::ResponseBody,
    bytes::Bytes,
    dev::service::Service,
    handler::{handler_service, request::RequestRef, state::StateRef},
    http::{Method, Uri, WebResponse},
    middleware::compress::Compress,
    route::Route,
    App, WebContext,
};

fn main() -> std::io::Result<()> {
    println!("open http://localhost:8080 in browser to visit the site");

    // use serve dir service as app state.
    App::with_state(ServeDir::new("static"))
        // catch all request path.
        .at(
            "/*path",
            // only accept get/post method.
            Route::new([Method::GET, Method::HEAD]).route(handler_service(index)),
        )
        // compression middleware
        .enclosed(Compress)
        // simple function middleware that intercept empty path and replace it with index.html
        .enclosed_fn(path)
        .serve()
        .bind("localhost:8080")?
        .run()
        .wait()
}

// extract request and serve dir state and start serving file.
async fn index(RequestRef(req): RequestRef<'_>, StateRef(dir): StateRef<'_, ServeDir>) -> WebResponse {
    match dir.serve(req).await {
        // map async file read stream to response body stream.
        Ok(res) => res.map(ResponseBody::box_stream),
        // map error to empty body response.
        Err(e) => e.into_response().map(|_| ResponseBody::bytes(Bytes::new())),
    }
}

// a type alias for web request with ServeDir as application state type attached to it.
type Context<'a> = WebContext<'a, ServeDir>;

async fn path<Res, Err>(
    service: &impl for<'r> Service<Context<'r>, Response = Res, Error = Err>,
    mut ctx: Context<'_>,
) -> Result<Res, Err> {
    match ctx.req().uri().path() {
        "/" | "" => *ctx.req_mut().uri_mut() = Uri::from_static("/index.html"),
        _ => {}
    }
    service.call(ctx).await
}
