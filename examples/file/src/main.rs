//! a low level static file serving example where the serve service is shared as application
//! state rather than a service type sit at fixed route endpoint.

use http_file::ServeDir;
use xitca_web::{
    dev::service::Service,
    handler::{handler_service, request::RequestRef, state::StateRef},
    http::Uri,
    request::WebRequest,
    response::{ResponseBody, StreamBody, WebResponse},
    route::get,
    App, HttpServer,
};

fn main() -> std::io::Result<()> {
    println!("open http://localhost:8080 in browser to visit the site");
    // construct a serve dir service
    let serve = ServeDir::new("static");
    HttpServer::new(move || {
        // use serve dir service as app state.
        App::with_multi_thread_state(serve.clone())
            // catch all request path.
            .at("/*path", get(handler_service(index)))
            // a simple middleware to intercept empty path and replace it with index.html
            .enclosed_fn(path)
            .finish()
    })
    .bind("localhost:8080")?
    .run()
    .wait()
}

// extract request and serve dir state and start serving file.
async fn index(RequestRef(req): RequestRef<'_>, StateRef(dir): StateRef<'_, ServeDir>) -> WebResponse {
    match dir.serve(req).await {
        Ok(res) => res.map(|body| ResponseBody::stream(StreamBody::new(body))),
        Err(e) => WebResponse::builder().status(400).body(format!("{e}").into()).unwrap(),
    }
}

// a type alias for web request with ServeDir as application state type attached to it.
type Request<'a> = WebRequest<'a, ServeDir>;

async fn path<Res, Err>(
    service: &impl for<'r> Service<Request<'r>, Response = Res, Error = Err>,
    mut req: Request<'_>,
) -> Result<Res, Err> {
    match req.req().uri().path() {
        "/" | "" => *req.req_mut().uri_mut() = Uri::from_static("/index.html"),
        _ => {}
    }
    service.call(req).await
}
