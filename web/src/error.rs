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

pub type Error<C> = Box<dyn for<'r> ErrorResponder<WebContext<'r, C>>>;

pub trait ErrorResponder<Req>: ResponderDyn<Req, Output = WebResponse> + error::Error + Send + Sync {}

impl<E, Req> ErrorResponder<Req> for E where E: ResponderDyn<Req, Output = WebResponse> + error::Error + Send + Sync {}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn trait_bound() {
        use crate::error::MatchError;
        let mut e = Box::new(MatchError::NotFound) as Error<()>;
        println!("{e:?}");
        println!("{e}");
        let mut ctx = WebContext::new_test(());
        let _ = e.respond_to(ctx.as_web_ctx());
        // TODO: stable trait upcast stables at 1.76
        // let e = &*e as &dyn std::error::Error;
        // e.downcast_ref::<MatchError>().unwrap();
    }
}
