//! middleware types.
//!
//! Middleware in xitca-web is powered by [Service] and [ServiceExt] trait.
//! [Service] trait provides logic for building and execute middleware.
//! [ServiceExt] trait provides combinator methods of applying middleware.
//!
//! # Quick start
//! ```rust
//! use xitca_web::{
//!     error::Error,
//!     http::WebResponse,
//!     handler::handler_service,
//!     service::{Service},
//!     App,
//!     WebContext
//! };
//!
//! // a typical middleware function boilerplate.
//! async fn middleware<S, C, B>(next: &S, ctx: WebContext<'_, C, B>) -> Result<WebResponse, Error<C>>
//! where
//!     S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Error<C>>
//! {
//!     // pre processing
//!     println!("request method: {}", ctx.req().method());
//!     
//!     // execute application service logic.
//!     let res = next.call(ctx).await?;
//!
//!     // post processing
//!     println!("response status code: {}", res.status());
//!     
//!     // return response
//!     Ok(res)
//! }
//!
//! App::new()
//!     .at("/", handler_service(handler))
//!     // apply middleware to all routes.
//!     .enclosed_fn(middleware);
//!
//! // place holder route handler.
//! async fn handler(_: &WebContext<'_>) -> &'static str {
//!     todo!()
//! }
//! ```
//!
//! # Generic types
//!
//! # Variants
//!
//! # Ordering
//! ```rust
//! use xitca_web::{
//!     error::Error,
//!     http::WebResponse,
//!     service::{fn_service, Service, ServiceExt},
//!     WebContext
//! };
//!
//! let service = fn_service(handler) // a service function
//!     .enclosed_fn(middleware_1) // enclose service function with middleware1
//!     .enclosed_fn(middleware_2); // enclose middleware1 with middleware2
//!
//! async fn handler(_: WebContext<'_>) -> Result<WebResponse, Error> {
//!     todo!()
//! }
//!
//! // placeholder middleware functions
//! async fn middleware_1<S>(next: &S, ctx: WebContext<'_>) -> Result<WebResponse, Error>
//! where
//!     S: for<'r> Service<WebContext<'r>, Response = WebResponse, Error = Error>
//! {
//!     todo!()
//! }
//!
//! async fn middleware_2<S>(next: &S, ctx: WebContext<'_>) -> Result<WebResponse, Error>
//! where
//!     S: for<'r> Service<WebContext<'r>, Response = WebResponse, Error = Error>
//! {
//!     todo!()
//! }
//! ```
//! Above code would produce a tree like ordering:
//! ```plain
//!  WebContext(original input)
//!             v   
//! +- middleware 2 ------------------------------------+
//! | WebContext(possible mutation)                     |
//! |             v                                     |
//! | +- middleware 1 --------------------------------+ |
//! | | WebContext(possible mutation)                 | |
//! | |             v                                 | |
//! | | +- handler --------------------------------+  | |
//! | | | WebContext > Result<WebResponse, Error>  |  | |
//! | | +------------------------------------------+  | |
//! | |             v                                 | |
//! | | Result<WebResponse, Error>(possible mutation) | |            
//! | +-----------------------------------------------+ |
//! |             v                                     |
//! | Result<WebResponse, Error>(possible mutation)     |
//! +---------------------------------------------------+
//!             v
//!  Result<WebResponse, Error>(final output)
//! ```
//! The main take away from the table should be:
//! - a type enclosed by middleware(s) is always the last one to take ownership of input request
//! and the first one to take ownership of output response (it's tasked with producing it)
//! - a middleware always take ownership of input before the type it enclosed on and always take
//! ownership of output response after
//! - multiple middlewares always go in a reverse order between input request and output
//! response: the input request goes from bottom to top.(in above example it goes from
//! `middleware2 -> middleware1 -> handler`). And the output response goes from top to bottom
//! (in above example it goes from `handler -> middleware1 -> middleware2`)
//! - `enclosed` and `enclosed_fn` share the same ordering rule however they are mixed in usage
//!
//! # Type mutation
//!
//! [Service]: crate::service::Service
//! [ServiceExt]: crate::service::ServiceExt

#[cfg(any(feature = "compress-br", feature = "compress-gz", feature = "compress-de"))]
pub mod compress;
#[cfg(any(feature = "compress-br", feature = "compress-gz", feature = "compress-de"))]
pub mod decompress;
#[cfg(feature = "tower-http-compat")]
pub mod tower_http_compat;

pub mod eraser;
pub mod limit;

#[cfg(not(target_family = "wasm"))]
pub mod sync;

pub use xitca_http::util::middleware::{Extension, Logger};
pub use xitca_service::middleware::{Group, UncheckedReady};

#[cfg(test)]
mod test {
    use xitca_http::body::RequestBody;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        handler::{extension::ExtensionRef, handler_service},
        http::{Request, RequestExt},
        service::Service,
        test::collect_string_body,
        App,
    };

    use super::*;

    #[test]
    fn extension() {
        async fn root(ExtensionRef(ext): ExtensionRef<'_, String>) -> String {
            ext.to_string()
        }

        let body = App::new()
            .at("/", handler_service(root))
            .enclosed(Extension::new("hello".to_string()))
            .enclosed(UncheckedReady)
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::new(RequestExt::<RequestBody>::default()))
            .now_or_panic()
            .unwrap()
            .into_body();

        let string = collect_string_body(body).now_or_panic().unwrap();
        assert_eq!(string, "hello");
    }
}
