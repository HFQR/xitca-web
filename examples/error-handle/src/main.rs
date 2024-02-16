#![feature(trait_upcasting)]

//! example of error handling in xitca-web.
//! code must be compiled with nightly Rust.

use std::{convert::Infallible, error, fmt};

use xitca_web::{
    error::Error,
    handler::handler_service,
    http::{StatusCode, WebResponse},
    route::get,
    service::Service,
    App, WebContext,
};

fn main() -> std::io::Result<()> {
    App::new()
        .at("/", get(handler_service(typed)))
        .at("/std", get(handler_service(std)))
        .enclosed_fn(error_handler)
        .serve()
        .bind("127.0.0.1:8080")?
        .run()
        .wait()
}

// a route that returns std::error::Error trait object.
// useful for use case where you don't care about typing your error in handler function.
async fn std() -> Result<&'static str, Box<dyn std::error::Error + Send + Sync>> {
    Err("std error".into())
}

// a route always return an error type that would produce 400 bad-request http response.
// see below MyError implements for more explanation
async fn typed() -> Result<&'static str, MyError> {
    Err(MyError::Index)
}

// an enum error type. must implement following traits:
// std::fmt::{Debug, Display} for formatting
// std::error::Error for backtrace and type casting
// xitca_web::dev::service::Service for http response generation.
#[derive(Debug)]
enum MyError {
    Index,
}

impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index => f.write_str("error from /"),
        }
    }
}

impl error::Error for MyError {}

// Error<C> is the main error type xitca-web uses and at some point MyError would
// need to be converted to it.
impl<C> From<MyError> for Error<C> {
    fn from(e: MyError) -> Self {
        Error::from_service(e)
    }
}

// response generator of MyError. in this case we generate blank bad request error.
impl<'r, C> Service<WebContext<'r, C>> for MyError {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C>) -> Result<Self::Response, Self::Error> {
        StatusCode::BAD_REQUEST.call(ctx).await
    }
}

// a middleware function used for intercept and interact with app handler outputs.
async fn error_handler<S, C, Res>(s: &S, ctx: WebContext<'_, C>) -> Result<Res, Error<C>>
where
    S: for<'r> Service<WebContext<'r, C>, Response = Res, Error = Error<C>>,
{
    s.call(ctx).await.map_err(|e| {
        // debug format error info.
        println!("{e:?}");

        // display format error info.
        println!("{e}");

        // utilize std::error::Error trait methods for backtrace and more advanced error info.
        let _source = e.source();

        // upcast trait and downcast to concrete type again.
        // this offers the ability to regain typed error specific error handling.
        // *. this is a runtime feature and not reinforced at compile time.
        if let Some(e) = (&*e as &dyn error::Error).downcast_ref::<MyError>() {
            match e {
                MyError::Index => {}
            }
        }

        e
    })
}

// if you prefer proc macro please reference examples/macros
