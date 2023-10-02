pub mod handler;
pub mod route;

mod context_priv;

pub mod context {
    pub use super::context_priv::{Context, ContextBuilder, ContextError};
}

#[cfg(feature = "router")]
mod router_priv;

#[cfg(feature = "router")]
pub mod router {
    pub use super::router_priv::{IntoObject, MatchError, Params, PathGen, Router, RouterError};
}

#[cfg(feature = "router")]
pub use router_priv::{Router, RouterError};
