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
//! async fn middleware_1<S, C>(next: &S, ctx: WebContext<'_, C>) -> Result<WebResponse, Error<C>>
//! where
//!     S: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Error<C>>
//! {
//!     next.call(ctx).await
//! }
//!
//! // you can also use a concrete type instead of generic in position of C but at the same time you would
//! // be facing compiler error if the application state is not the same type
//! async fn middleware_2<S>(next: &S, ctx: WebContext<'_, String>) -> Result<WebResponse, Error<String>>
//! where
//!     S: for<'r> Service<WebContext<'r, String>, Response = WebResponse, Error = Error<String>>
//! {
//!     next.call(ctx).await
//! }
//! 
//! // a middleware expecting usize as application state.
//! async fn middleware_3<S>(next: &S, ctx: WebContext<'_, usize>) -> Result<WebResponse, Error<usize>>
//! where
//!     S: for<'r> Service<WebContext<'r, usize>, Response = WebResponse, Error = Error<usize>>
//! {
//!     next.call(ctx).await
//! }
//!
//! App::with_state(String::from("996")) // construct an application with String as typed state.
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
//! For detailed explanation of application state please reference [App::with_state]
//!
//! ## Generic type for http body
//! ```rust
//! # use xitca_web::{error::Error, handler::handler_service, http::WebResponse, service::Service, App, WebContext};
//! // from above example we added generic type B to WebContext an B type is type of http body.
//! // in xitca-web http body is a dynamic type that can be mutated and transformed and marking it generic
//! // in your middleware function would make it more flexible to adapt type changes in your application
//! // compose. But it's not required and in most case it can be leaved as default by not writing it out.
//! async fn middleware_1<S, C, B>(next: &S, ctx: WebContext<'_, C, B>) -> Result<WebResponse, Error<C>>
//! where
//!     S: for<'r> Service<WebContext<'r, C, B>, Response = WebResponse, Error = Error<C>>
//! {
//!     next.call(ctx).await
//! }
//!
//! // since the application we are building does not do any type mutation we can also don't annotate B type
//! async fn middleware_2<S, C>(next: &S, ctx: WebContext<'_, C>) -> Result<WebResponse, Error<C>>
//! where
//!     S: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Error<C>>
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
//! For http body type mutation please reference the `Type Mutation` part below.
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
//! [App::with_state]: crate::App::with_state
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
