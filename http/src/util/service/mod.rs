pub mod handler;
pub mod route;

mod context_priv;
mod router_priv;

pub mod context {
    pub use super::context_priv::{object, Context, ContextBuilder, ContextError};
}

pub mod router {
    pub use super::router_priv::{GenericRouter, MatchError, Params, PathGen, Router, RouterError};
}

pub use router_priv::{GenericRouter, Router, RouterError};
