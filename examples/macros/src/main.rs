//! example of utilizing macro for routing service.

use xitca_web::{
    bytes::Bytes,
    codegen::{error_impl, route},
    handler::state::StateRef,
    http::{StatusCode, WebResponse},
    App, WebContext,
};

fn main() -> std::io::Result<()> {
    App::with_state(String::from("Hello,World!"))
        .at_typed(root)
        .at_typed(private)
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

#[route("/", method = get)]
async fn root(StateRef(s): StateRef<'_, String>) -> String {
    s.to_owned()
}

// a private endpoint always return an error.
// please reference examples/error-handle for erroring handling pattern in xitca-web
#[route("/private", method = get)]
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
