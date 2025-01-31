//! middleware types.
//!
//! Middleware in xitca-web is powered by [`Service`] and [`ServiceExt`] trait.
//! [`Service`] trait provides logic for building and execute middleware.
//! [`ServiceExt`] trait provides combinator methods of applying middleware.
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
//! async fn middleware<S, C, B>(next: &S, ctx: WebContext<'_, C, B>) -> Result<WebResponse, Error>
//! where
//!     S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Error>
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
//! # Types in middleware
//! In quick start example there are multiple generic/concrete types annotate on the middleware function.
//! Below is a break down of these types and if some types are missing in certain section it's due to
//! not all these types have to be explicitly annotated and some of them can be added/removed according to
//! specific use case.
//!
//! ## Generic type for application service and common concrete types
//! ```rust
//! # use xitca_web::{error::Error, http::WebResponse, service::Service, WebContext};
//! // S type here is generic type for application service and possible nested middlewares.
//! // It has to be generic as the composed final type can be vary according to how application service is built.
//! // WebResponse and Error are the suggested concert types and you can just always write them as is.
//! async fn middleware<S>(next: &S, ctx: WebContext<'_>) -> Result<WebResponse, Error>
//! where
//!     // The S type has to be constraint with Service trait so we can call it's Service::call method to execute
//!     // it's logic.
//!     S: for<'r> Service<WebContext<'r>, Response = WebResponse, Error = Error>
//! {
//!     // the middleware does nothing but execute the Service::call of application service
//!     next.call(ctx).await
//! }
//! ```
//!
//! ## Generic type for application state
//! ```rust
//! # use xitca_web::{error::Error, handler::handler_service, http::WebResponse, service::Service, App, WebContext};
//! // from above example we added generic type C to WebContext and Error type and C is type of application state.
//! // in xitca-web application state is compile time type checked therefore in middleware you have to annotate
//! // the type to comply.
//! async fn middleware_1<S, C>(next: &S, ctx: WebContext<'_, C>) -> Result<WebResponse, Error>
//! where
//!     S: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Error>
//! {
//!     next.call(ctx).await
//! }
//!
//! // you can also use a concrete type instead of generic in position of C but at the same time you would
//! // be facing compiler error if the application state is not the same type
//! async fn middleware_2<S>(next: &S, ctx: WebContext<'_, String>) -> Result<WebResponse, Error>
//! where
//!     S: for<'r> Service<WebContext<'r, String>, Response = WebResponse, Error = Error>
//! {
//!     next.call(ctx).await
//! }
//!
//! // a middleware expecting usize as application state.
//! async fn middleware_3<S>(next: &S, ctx: WebContext<'_, usize>) -> Result<WebResponse, Error>
//! where
//!     S: for<'r> Service<WebContext<'r, usize>, Response = WebResponse, Error = Error>
//! {
//!     next.call(ctx).await
//! }
//!
//! App::new()
//!     .with_state(String::from("996")) // construct an application with String as typed state.
//!     # .at("/", handler_service(handler))
//!     .enclosed_fn(middleware_1)
//!     // uncomment the line below would cause a compile error. because middleware_3 expecting usize
//!     // type as application state but the App has a state type of String.
//!     // .enclosed_fn(middleware_3)
//!     .enclosed_fn(middleware_2); // both generic and concrete typed middleware would work.
//!
//! # async fn handler<C>(_: &WebContext<'_, C>) -> &'static str {
//! #   todo!()
//! # }
//! ```
//! For detailed explanation of application state please reference [`App::with_state`]
//!
//! ## Generic type for http body
//! ```rust
//! # use xitca_web::{error::Error, handler::handler_service, http::WebResponse, service::Service, App, WebContext};
//! // from above example we added generic type B to WebContext an B type is type of http body.
//! // in xitca-web http body is a dynamic type that can be mutated and transformed and marking it generic
//! // in your middleware function would make it more flexible to adapt type changes in your application
//! // compose. But it's not required and in most case it can be leaved as default by not writing it out.
//! async fn middleware_1<S, C, B>(next: &S, ctx: WebContext<'_, C, B>) -> Result<WebResponse, Error>
//! where
//!     S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Error>
//! {
//!     next.call(ctx).await
//! }
//!
//! // since the application we are building does not do any type mutation we can also don't annotate B type
//! async fn middleware_2<S, C>(next: &S, ctx: WebContext<'_, C>) -> Result<WebResponse, Error>
//! where
//!     S: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Error>
//! {
//!     next.call(ctx).await
//! }
//!
//! App::new()
//!     # .at("/", handler_service(handler))
//!     .enclosed_fn(middleware_1)
//!     .enclosed_fn(middleware_2); // both generic and concrete typed middleware would work.
//!
//! # async fn handler(_: &WebContext<'_>) -> &'static str {
//! #   todo!()
//! # }
//! ```
//! For http body type mutation please reference the [`Type Mutation`](#type-mutation) part below.
//!
//! # Variants
//! ## async function as middleware
//! Please reference `Types in middleware` part above
//! ## named type as middleware
//! ```rust
//! # use xitca_web::{error::Error, handler::handler_service, http::WebResponse, service::Service, App, WebContext};
//! // in xitca-web a named type middleware is constructed with a two step process:
//! // 1. a builder type and trait impl for constructing the middleware service.
//! // 2. a runner type and trait impl for running the middleware service.
//!
//! // a named type for building the middleware service. it's a blueprint of constructing service
//! // that can be repeatedly utilized to produce multiple running services.
//! struct Middleware;
//!
//! // trait impl for Middleware type where the logic of building middleware lives.
//! // generic S type represent the running service type Middleware enclosed upon and in this example's case it's
//! // the application service type.
//! // generic E type represent the possible error when building application. our builder logic can interact with the
//! // error and mutate it. in this case we just forward the error in output as our middleware builder is infallible.
//! impl<S, E> Service<Result<S, E>> for Middleware {
//!     // response type represent the successful built running service type.
//!     type Response = MiddlewareService<S>;
//!     // error type represent the possible error in building process.
//!     type Error = E;
//!
//!     async fn call(&self, arg: Result<S, E>) -> Result<Self::Response, Self::Error> {
//!         // the building logic of middleware services lives here and in this case it's a simple new type around S
//!         arg.map(MiddlewareService)       
//!     }
//! }
//!
//! // a named type for running middleware service. the Middleware type above would build this type when application
//! // starts up and the running service is what interact with client http requests.
//! // generic S type represent the running service type Middleware enclosed upon and in this example's case it's
//! // the application service type.
//! struct MiddlewareService<S>(S);
//!
//! // trait impl for MiddlewareService type where the logic of running middleware lives.
//! // please reference `Types in middleware` session above for explanation of types used here.
//! // function middleware and type middleware share the same types when interacting with http types.
//! impl<'r, S, C, B> Service<WebContext<'r, C, B>> for MiddlewareService<S>
//! where
//!     S: for<'r2> Service<WebContext<'r2, C, B>, Response = WebResponse, Error = Error>
//! {
//!     type Response = WebResponse;
//!     type Error = Error;
//!
//!     async fn call(&self, req: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
//!         // logic for running middleware service lives here. in this case it just forward to application service
//!         self.0.call(req).await
//!     }
//! }
//!
//! App::new()
//!     # .at("/", handler_service(handler))
//!     .enclosed(Middleware); // apply middleware to application
//!
//! # async fn handler(_: &WebContext<'_>) -> &'static str {
//! #   todo!()
//! # }
//! ```
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
//!          WebContext(original input)
//!                      v   
//! +------------- middleware 2 ------------------------+
//! |        WebContext(possible mutation)              |
//! |                    v                              |
//! | +----------- middleware 1 ----------------------+ |
//! | |      WebContext(possible mutation)            | |
//! | |                  v                            | |
//! | | +--------- handler -------------------------+ | |
//! | | |  WebContext > Result<WebResponse, Error>  | | |
//! | | +-------------------------------------------+ | |
//! | |                  v                            | |
//! | | Result<WebResponse, Error>(possible mutation) | |            
//! | +-----------------------------------------------+ |
//! |                    v                              |
//! |   Result<WebResponse, Error>(possible mutation)   |
//! +---------------------------------------------------+
//!                      v
//!     Result<WebResponse, Error>(final output)
//! ```
//! The main take away from the table should be:
//! - a type enclosed by middleware(s) is always the last one to take ownership of input request
//!   and the first one to take ownership of output response (it's tasked with producing it)
//! - a middleware always take ownership of input before the type it enclosed on and always take
//!   ownership of output response after
//! - multiple middlewares always go in a reverse order between input request and output
//!   response: the input request goes from bottom to top.(in above example it goes from
//!   `middleware2 -> middleware1 -> handler`). And the output response goes from top to bottom
//!   (in above example it goes from `handler -> middleware1 -> middleware2`)
//! - `enclosed` and `enclosed_fn` share the same ordering rule however they are mixed in usage
//!
//! # Type mutation
//! - In [`WebContext<'_, C, B>`] type the last generic type param `B` is for http body type.
//!   middleware and service are free to transform it's type and constraint it's inner/next service
//!   to accept the new type as it's request http body type. [`DeCompress`] and [`Limit`] middleware are
//!   examples of this practice. [`TypeEraser`] middleware on the other hand can be used to reserve the
//!   type mutation and restore `B` type to it's default as [`RequestBody`] type. In this case web context
//!   can be written in short form as [`WebContext<'_, C>`].
//! - [`WebResponse<B>`] type share the characteristic as web context type. The `B` type can be transform
//!   into new type by services and middleware while type eraser is able to reverse the process.
//!
//! [`App::with_state`]: crate::App::with_state
//! [`Service`]: crate::service::Service
//! [`ServiceExt`]: crate::service::ServiceExt
//! [`WebContext<'_, C, B>`]: crate::WebContext
//! [`WebContext<'_, C>`]: crate::WebContext
//! [`Decompress`]: crate::middleware::decompress::Decompress
//! [`Limit`]: crate::middleware::limit::Limit
//! [`TypeEraser`]: crate::middleware::eraser::TypeEraser
//! [`RequestBody`]: crate::body::RequestBody
//! [`WebResponse<B>`]: crate::http::WebResponse

#[cfg(any(feature = "compress-br", feature = "compress-gz", feature = "compress-de"))]
pub mod compress;
#[cfg(any(feature = "compress-br", feature = "compress-gz", feature = "compress-de"))]
pub mod decompress;
#[cfg(feature = "rate-limit")]
pub mod rate_limit;
#[cfg(not(target_family = "wasm"))]
pub mod sync;
#[cfg(feature = "tower-http-compat")]
pub mod tower_http_compat;

pub mod eraser;
pub mod limit;

#[cfg(feature = "logger")]
mod logger;
#[cfg(feature = "logger")]
pub use logger::Logger;

mod catch_unwind;
mod context;

pub use catch_unwind::CatchUnwind;
pub use context::WebContext;
pub use xitca_http::util::middleware::Extension;
pub use xitca_service::middleware::{AsyncFn, Group, UncheckedReady};

#[cfg(test)]
mod test {
    use xitca_http::body::RequestBody;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        App,
        handler::{extension::ExtensionRef, handler_service},
        http::{Request, RequestExt},
        service::Service,
        test::collect_string_body,
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
