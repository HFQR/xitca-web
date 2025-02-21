//! a web framework focus on memory efficiency, composability, and fast compile time.
//!
//! # Quick start
//! ```no_run
//! use xitca_web::{handler::handler_service, route::get, App};
//!
//! fn main() -> std::io::Result<()> {
//!     App::new()
//!         .at("/", get(handler_service(|| async { "Hello,World!" })))
//!         .serve()
//!         .bind("localhost:8080")?
//!         .run()
//!         .wait()
//! }
//! ```
//!
//! # Memory efficient
//! - zero copy magic types
//! - zero cost service tree
//!
//! ## Zero copy
//! ```rust
//! # #[cfg(feature = "json")]
//! # fn _main() -> std::io::Result<()> {
//! use xitca_web::{
//!     error::Error,
//!     handler::{handler_service, json::LazyJson, path::PathRef},
//!     route::{get, post},
//!     App
//! };
//!
//! // PathRef is able to borrow http request's path string as reference
//! // without copying it.
//! async fn url_query(PathRef(path): PathRef<'_>) -> &'static str {
//!     println!("{path}");
//!     "zero copy path"    
//! }
//!
//! // deserializable user type.
//! #[derive(serde::Deserialize)]
//! struct User<'a> {
//!     name: &'a str
//! }
//!
//! // LazyJson is able to lazily deserialize User type with zero copy &str.
//! async fn json(lazy: LazyJson<User<'_>>) -> Result<&'static str, Error> {
//!     let User { name } = lazy.deserialize()?;
//!     println!("{name}");
//!     Ok("zero copy json")    
//! }
//!
//! // Almost all magic extract types in xitca-web utilize zero copy
//! // to avoid unnecessary memory copy.
//! App::new()
//!     // a route handling incoming url query.
//!     .at("/query", get(handler_service(url_query)))
//!     // a route handling incoming json object.
//!     .at("/json", post(handler_service(json)))
//!     .serve()
//!     .bind("localhost:8080")?
//!     .run()
//!     .wait()
//! # }
//! ```
//!
//! ## Zero cost
//! ```rust
//! use xitca_web::{
//!     handler::{handler_service},
//!     http::WebResponse,
//!     route::get,
//!     middleware::Extension,
//!     service::{Service, ServiceExt},
//!     App, WebContext
//! };
//! # fn _main() -> std::io::Result<()> {
//! App::new()
//!     .at("/", get(handler_service(|| async { "hello,world!" })))
//!     // ServiceExt::enclosed_fn combinator enables async function as middleware.
//!     // the async function is unboxed and potentially inlined with other async services
//!     // for efficient binary code with less memory allocation.
//!     .enclosed_fn(middleware_fn)
//!     // ServiceExt::enclosed combinator enables type impl Service trait as middleware.
//!     // the middleware trait method is unboxed and potentially inlined with other async services
//!     // for efficient binary code with less memory allocation.
//!     .enclosed(Extension::new(()))
//!     .serve()
//!     .bind("localhost:8080")?
//!     .run()
//!     .wait()
//! # }
//!
//! // a simple middleware just forward request to inner service logic.
//! async fn middleware_fn<S, T, E>(service: &S, ctx: WebContext<'_>) -> Result<T, E>
//! where
//!     S: for<'r> Service<WebContext<'r>, Response = T, Error = E>
//! {
//!     service.call(ctx).await
//! }
//! ```
//!
//! # Composable
//! - Easy mixture of various level of abstractions and less opinionated APIs
//! - Common types and traits for cross crates integration of majority rust web ecosystem
//!
//! ## Abstraction variety
//! ```rust
//! use xitca_web::{
//!     body::ResponseBody,
//!     error::Error,
//!     handler::{handler_service, handler_sync_service, Responder},
//!     http::{Method, WebResponse},
//!     route::get,
//!     service::fn_service,
//!     App, WebContext
//! };
//!
//! # fn _main() -> std::io::Result<()> {
//! App::new()
//!     // high level abstraction. see fn high for detail.
//!     .at("/high", get(handler_service(high)))
//!     // low level abstraction. see fn low for detail.
//!     .at("/low", get(fn_service(low)))
//!     // abstraction for synchronous. see fn sync for detail.
//!     .at("/sync", get(handler_sync_service(sync)))
//!     .serve()
//!     .bind("localhost:8080")?
//!     .run()
//!     .wait()
//! # }
//!
//! // magic function with arbitrary receiver type and output type
//! // that can be extracted from http requests and packed into http
//! // response.
//! async fn high(method: &Method) -> &'static str {
//!     // extract http method from http request.
//!     assert_eq!(method, Method::GET);
//!     // pack string literal into http response.
//!     "high level"     
//! }
//!
//! // function with concrete typed input and output where http types
//! // are handled manually and explicitly.
//! // it can also be seen as a desugar of previous example of
//! // handler_service(high)
//! async fn low(ctx: WebContext<'_>) -> Result<WebResponse, Error> {
//!     // extract method from request context.
//!     let method = ctx.extract().await?;
//!     // execute high level abstraction example function.
//!     let str = high(method).await;
//!     // convert string literal to http response.
//!     str.respond(ctx).await
//! }
//!
//! // high level abstraction but for synchronous function. this function
//! // is powered by background thread pool so it does not block the async
//! // code.
//! fn sync(method: Method) -> &'static str {
//!     assert_eq!(method, Method::GET);
//!     // blocking thread for long period of time does not impact xitca-web
//!     // async internal.
//!     std::thread::sleep(std::time::Duration::from_secs(3));
//!     "sync"    
//! }
//! ```
//!
//! ## Middleware composability
//! ```rust
//! use xitca_web::{
//!     error::Error,
//!     handler::{handler_service},
//!     http::WebResponse,
//!     route::get,
//!     service::{Service, ServiceExt},
//!     App, WebContext
//! };
//!
//! # fn _main() -> std::io::Result<()> {
//! // ServiceExt::enclosed_fn combinator enables async function as middleware.
//! // in xitca_web almost all service can be enclosed by an middleware.
//! App::new()
//!     .at("/",
//!         get(
//!             // apply middleware to handler_service
//!             handler_service(|| async { "hello,world!" })
//!                 .enclosed_fn(middleware_fn)
//!         )
//!         // apply middleware to route
//!         .enclosed_fn(middleware_fn)
//!     )
//!     // apply middleware to application
//!     .enclosed_fn(middleware_fn)
//!     .serve()
//!     .bind("localhost:8080")?
//!     .run()
//!     .wait()
//! # }
//!
//! // a simple middleware just forward request to inner service logic.
//! async fn middleware_fn<S, T, E>(service: &S, ctx: WebContext<'_>) -> Result<T, E>
//! where
//!     S: for<'r> Service<WebContext<'r>, Response = T, Error = E>
//! {
//!     service.call(ctx).await
//! }
//! ```
//! For more detailed middleware documentation please reference [middleware]
//!
//! ## Cross crates integration
//! use tower-http inside xitca-web application.
//! ```rust
//! # #[cfg(feature = "tower-http-compat")]
//! # fn _main() -> std::io::Result<()> {
//! use tower_http::services::ServeDir;
//! use xitca_web::{
//!     service::tower_http_compat::TowerHttpCompat,
//!     App
//! };
//!
//! App::new()
//!     .at("/", TowerHttpCompat::new(ServeDir::new("/some_folder")))
//!     .serve()
//!     .bind("localhost:8080")?
//!     .run()
//!     .wait()
//! # }
//! ```
//!
//! # Fast compile time
//! - additive proc macro
//! - light weight dependency tree
//!
//! ## opt-in proc macro
//! in xitca-web proc macro is opt-in. This result in a fast compile time with zero
//! public proc macro. That said you still can enable macros for a higher level style
//! of API.
//! ```rust
//! # #[cfg(feature = "codegen")]
//! # async fn _main() -> std::io::Result<()> {
//! use xitca_web::{codegen::route, App};
//!
//! #[route("/", method = get)]
//! async fn index() -> &'static str {
//!     "Hello,World!"
//! }
//!
//! App::new()
//!     .at_typed(index)
//!     .serve()
//!     .bind("localhost:8080")?
//!     .run()
//!     .await
//! # }
//! ```

#![forbid(unsafe_code)]
#![cfg_attr(feature = "nightly", feature(async_iterator, error_generic_member_access))]

mod app;
mod context;
#[cfg(feature = "__server")]
mod server;

pub mod body;
pub mod error;
pub mod handler;
pub mod middleware;
pub mod service;
pub mod test;

#[cfg(feature = "codegen")]
pub mod codegen {
    //! macro code generation module.

    /// Derive macro for individual struct field type extractable through [StateRef](crate::handler::state::StateRef)
    /// and [StateOwn](crate::handler::state::StateOwn)
    ///
    /// # Example:
    /// ```rust
    /// # use xitca_web::{codegen::State, handler::{handler_service, state::StateRef}, App, WebContext};
    ///
    /// // use derive macro and attribute to mark the field that can be extracted.
    /// #[derive(State, Clone)]
    /// struct MyState {
    ///     #[borrow]
    ///     field: u128
    /// }
    ///
    /// # async fn app() {
    /// // construct App with MyState type.
    /// App::new()
    ///     .with_state(MyState { field: 996 })
    ///     .at("/", handler_service(index))
    /// #   .at("/nah", handler_service(nah));
    /// # }
    ///
    /// // extract u128 typed field from MyState.
    /// async fn index(StateRef(num): StateRef<'_, u128>) -> String {
    ///     assert_eq!(*num, 996);
    ///     num.to_string()
    /// }
    /// # async fn nah(_: &WebContext<'_, MyState>) -> &'static str {
    /// #   // needed to infer the body type of request
    /// #   ""
    /// # }
    /// ```
    pub use xitca_codegen::State;

    pub use xitca_codegen::route;

    pub use xitca_codegen::error_impl;

    #[doc(hidden)]
    /// a hidden module for macro to access public types that are not framework user facing.
    pub mod __private {
        pub use xitca_http::util::service::router::{IntoObject, RouteObject, RouterMapErr, TypedRoute};
    }
}

pub mod http {
    //! http types

    use super::body::{RequestBody, ResponseBody};

    pub use xitca_http::http::*;

    /// type alias for default request type xitca-web uses.
    pub type WebRequest<B = RequestBody> = Request<RequestExt<B>>;

    /// type alias for default response type xitca-web uses.
    pub type WebResponse<B = ResponseBody> = Response<B>;
}

pub mod route {
    //! route services.
    //! # Examples:
    //! ```
    //! # use xitca_web::{
    //! #   handler::handler_service,
    //! #   http::Method,
    //! #   route::{get, post, put, Route}
    //! # };
    //! # fn _route() -> Result<(), Box<dyn std::error::Error>> {
    //! // a simple async function service.
    //! let service = handler_service(|_: ()| async { });
    //!
    //! let custom_method = Method::from_bytes(b"huh")?;
    //!
    //! // route get http method and a user custom method to handler_service
    //! Route::new([Method::GET, custom_method]).route(service.clone());
    //!
    //! // shortcut for single http method route(s). they can chained multiple times.
    //! get(service.clone()).post(service.clone()).put(service);
    //! # Ok(())
    //! # }
    //! ```
    pub use xitca_http::util::service::route::{Route, connect, delete, get, head, options, patch, post, put, trace};
}

pub use app::{App, AppObject, NestApp};
pub use body::BodyStream;
pub use context::WebContext;
#[cfg(feature = "__server")]
pub use server::HttpServer;

pub use xitca_http::bytes;
