//! traits for composable async functions.
//!
//! # Examples
//! ```rust
//! use core::convert::Infallible;
//!
//! use xitca_service::{fn_service, Service, ServiceExt};
//! # async fn call() -> Result<(), Infallible> {
//!
//! // a middleware function that has ownership of the argument and output of S as Service
//! // trait implementor.
//! async fn middleware<S>(s: &S, req: String) -> Result<String, Infallible>
//! where
//!     S: Service<String, Response = String, Error = Infallible>
//! {
//!     let req2 = req.clone();
//!     let mut res = s.call(req).await?;
//!     assert_eq!(res, req2);
//!     res.push_str("-dagongren");
//!     Ok(res)
//! }
//!
//! // apply middleware to async function as service.
//! let builder = fn_service(|req: String| async { Ok::<_, Infallible>(req) })
//!     .enclosed_fn(middleware);
//!
//! // build the composited service.
//! let service = builder.call(()).await?;
//!
//! // execute the service function with string argument.
//! let res = service.call("996".to_string()).await?;
//!
//! assert_eq!(res, "996-dagongren");
//!
//! # Ok(()) }
//! ```
#![no_std]
#![forbid(unsafe_code)]

mod service;

pub mod middleware;
pub mod pipeline;
pub mod ready;

pub use self::{
    pipeline::{EnclosedBuilder, EnclosedFnBuilder, MapBuilder, MapErrorBuilder},
    service::{FnService, Service, ServiceExt, fn_build, fn_service},
};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
pub mod object;

#[cfg(feature = "alloc")]
/// boxed [core::future::Future] trait object with no extra auto trait bound(`!Send` and `!Sync`).
pub type BoxFuture<'a, Res, Err> =
    core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = Result<Res, Err>> + 'a>>;
