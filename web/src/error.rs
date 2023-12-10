//! web error types.

use std::error;

use xitca_http::util::service::handler::ResponderDyn;

pub use xitca_http::{
    error::BodyError,
    util::service::{
        route::{MethodNotAllowed, RouteError},
        router::{MatchError, RouterError},
    },
};

use super::{context::WebContext, http::WebResponse};

/// type erased error with additional formatting, type casting and [WebResponse] generation logic.
///
/// # Examples:
/// ```rust
/// # use xitca_http::util::service::handler::Responder;
/// use xitca_web::{error::Error, http::WebResponse, WebContext};
///
/// async fn error_handle<C>(e: Error<C>, ctx: WebContext<'_, C>) -> WebResponse {
///     // debug format error
///     println!("{e:?}");
///
///     // display format error
///     println!("{e}");
///
///     // note: trait upcasting feature is stabled in Rust 1.76.
///     // type downcast for specified typed error handling.
///     // if let Some(_) = (&*e as &dyn std::error::Error).downcast_ref::<std::convert::Infallible>() {
///     // }
///
///     // generate http response from error
///     e.respond_to(ctx).await
/// }
/// ```
pub type Error<C> = Box<dyn for<'r> ErrorResponder<WebContext<'r, C>>>;

pub trait ErrorResponder<Req>: ResponderDyn<Req, Output = WebResponse> + error::Error + Send + Sync {}

impl<E, Req> ErrorResponder<Req> for E where E: ResponderDyn<Req, Output = WebResponse> + error::Error + Send + Sync {}
