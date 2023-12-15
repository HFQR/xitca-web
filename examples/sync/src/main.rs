//! sync version of hello-world example in this repo.

use xitca_web::{
    handler::handler_sync_service,
    http::{WebRequest, WebResponse},
    middleware::sync::{Next, SyncMiddleware},
    route::get,
    App,
};

fn main() -> std::io::Result<()> {
    App::new()
        .at("/", get(handler_sync_service(|| "Hello,World!")))
        .enclosed(SyncMiddleware::new(middleware_fn))
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

// a simple function forward request to the next service in service tree.
// in this case it's the app service and it's router and handlers.
// generic E type is the potential error type produced by next service.
fn middleware_fn<E>(req: WebRequest<()>, next: &mut Next<E>) -> Result<WebResponse<()>, E> {
    // before calling the next service we can access and/or mutate the state of request.
    let res = next.call(req);
    // after the next service call return we can access and/or mutate the output.
    res
}
