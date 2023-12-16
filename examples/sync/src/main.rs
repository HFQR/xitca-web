//! sync version of hello-world example in this repo.

use xitca_web::{
    handler::handler_sync_service,
    http::WebResponse,
    middleware::sync::{Next, SyncMiddleware},
    route::get,
    App, WebContext,
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

// a simple function forward request to the next service in service tree:
// in this case it's the app service and it's router and handlers.
// generic E type is the potential error type produced by app service.
// generic C type is the type state of App.
fn middleware_fn<E, C>(next: &mut Next<E>, ctx: WebContext<'_, C>) -> Result<WebResponse<()>, E> {
    // before calling the next service we can access and/or mutate the state of context.
    let res = next.call(ctx); // call next service.
                              // after the next service call return we can access and/or mutate the output.
    res
}
