//! example of error handling in xitca-web.
//! code must be compiled with nightly Rust.

// nightly rust features are not required for error handling in xitca-web. but they can enhance
// the experience quite a bit with certain features and they are purely opt-in.
//
// error_generic_member_access and error_reporter nightly feature enables stack backtrace capture and display.
#![feature(error_generic_member_access, error_reporter)]

use std::{backtrace::Backtrace, convert::Infallible, error, fmt};

use xitca_web::{
    error::{Error, MatchError},
    handler::{handler_service, html::Html, Responder},
    http::{StatusCode, WebResponse},
    route::get,
    service::Service,
    App, WebContext,
};

fn main() -> std::io::Result<()> {
    App::new()
        // "http:://127.0.0.1:8080/" would respond with blank 400 http response and print error format with thread backtrace.
        .at("/", get(handler_service(typed)))
        // "http:://127.0.0.1:8080/std" would respond with blank 500 http response and print error format.
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
    // forcefully capture thread backtrace. see error_handler function below and error::Report
    // for it's usage.
    Err(MyError {
        backtrace: Backtrace::force_capture(),
    })
}

// a custom error type. must implement following traits:
// std::fmt::{Debug, Display} for formatting
// std::error::Error for backtrace and type casting
// From for converting from Self to xitca_web::error::Error type.
// xitca_web::service::Service for lazily generating http response.
struct MyError {
    // thread backtrace which can be requested after type erasing through std::error::Error::provide API.
    backtrace: Backtrace,
}

impl fmt::Debug for MyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MyError").finish()
    }
}

impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("custom error")
    }
}

impl error::Error for MyError {
    // necessary for providing backtrace to xitca_web::error::Error instance.
    fn provide<'a>(&'a self, request: &mut error::Request<'a>) {
        request.provide_ref(&self.backtrace);
    }
}

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
async fn error_handler<S, C>(s: &S, mut ctx: WebContext<'_, C>) -> Result<WebResponse, Error<C>>
where
    S: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Error<C>>,
{
    match s.call(ctx.reborrow()).await {
        Ok(res) => Ok(res),
        Err(e) => {
            // debug format error info.
            println!("{e:?}");

            // display format error info.
            println!("{e}");

            // generate http response actively. from here it's OK to early return it in Result::Ok
            // variant as error_handler function's output
            let _res = e.call(ctx.reborrow()).await?;
            // return Ok(_res);

            // above are error handling enabled by stable rust.

            // below are error handling enabled by nightly rust.

            // utilize std::error module for backtrace and more advanced error info.
            let report = error::Report::new(&e).pretty(true).show_backtrace(true);
            // display error report
            println!("{report}");

            // upcast trait and downcast to concrete type again.
            // this offers the ability to regain typed error specific error handling.
            // *. this is a runtime feature and not reinforced at compile time.
            if let Some(_e) = e.upcast().downcast_ref::<MyError>() {
                // handle typed error.
            }

            // type casting can also be used to handle xitca-web's "internal" error types for overriding
            // default error behavior.
            // *. "internal" means these error types have their default error formatter and http response generator.
            // *. "internal" error types are public types exported through `xitca_web::error` module. it's OK to
            // override them for custom formatting/http response generating.
            if e.upcast().downcast_ref::<MatchError>().is_some() {
                // MatchError is error type for request not matching any route from application service.
                // in this case we override it's default behavior by generating a different http response.
                return (Html("<h1>404 Not Found</h1>"), StatusCode::NOT_FOUND)
                    .respond(ctx)
                    .await;
            }

            // the most basic error handling is to ignore it and return as is. xitca-web is able to take care
            // of error by utilizing it's according trait implements(Debug,Display,Error and Service impls)
            Err(e)
        }
    }
}

// if you prefer proc macro please reference examples/macros where the trait implements can be drastically easier
// at the cost of some compile time.
