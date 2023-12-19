//! example of utilizing macro for routing service and handling error.

use xitca_web::{
    bytes::Bytes,
    codegen::{error_impl, route},
    handler::state::{StateOwn, StateRef},
    http::{StatusCode, WebResponse},
    service::Service,
    App, WebContext,
    middleware::Logger
};

fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("[xitca-logger]=trace")
        .init();
    App::with_state(String::from("Hello,World!"))
        .at_typed(root)
        .at_typed(sync)
        .at_typed(private)
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

// a simple middleware function forward request to other services and display response status code.
async fn middleware_fn<S, C, B, Err>(s: &S, ctx: WebContext<'_, C, B>) -> Result<WebResponse, Err>
where
    S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Err>,
{
    s.call(ctx).await.map(|res| {
        tracing::info!("response status: {}", res.status());
        res
    })
}

// decorate async function handler with route macro.
#[route(
    // string path of route that registered wit router. incoming http request would match it's uri against the path.
    "/", 
    // http method of route that registered with route.
    method = get,
    // enclosed async function handler with given middleware function.
    enclosed_fn = middleware_fn,
    // multiple middlewares can be applied.
    enclosed_fn = middleware_fn,
    // typed middleware can be used though the constructor must be in place of the attribute.
    enclosed = Logger::new()
)]
async fn root(StateRef(s): StateRef<'_, String>) -> String {
    s.to_owned()
}

// routing sync function with thread pooling.
#[route("/sync", method = get)]
fn sync(StateOwn(s): StateOwn<String>) -> String {
    s
}

// a private endpoint always return an error.
// please reference examples/error-handle for erroring handling pattern in xitca-web
#[route("/private", method = get, enclosed = Logger::new())]
async fn private() -> Result<WebResponse, MyError> {
    Err(MyError::Private)
}

// derive error type with thiserror for Debug, Display and Error trait impl
#[derive(thiserror::Error, Debug)]
enum MyError {
    #[error("error from /private")]
    Private,
}

// error_impl is an attribute macro for http response generation
#[error_impl]
impl MyError {
    // generate a blank 400 response.
    async fn call<C>(&self, ctx: WebContext<'_, C>) -> WebResponse {
        let mut res = ctx.into_response(Bytes::new());
        *res.status_mut() = StatusCode::BAD_REQUEST;
        res
    }
}
