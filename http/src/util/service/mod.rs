pub mod handler;
pub mod route;

#[cfg(feature = "router")]
mod router_priv;

#[cfg(feature = "router")]
pub mod router {
    pub use super::router_priv::{IntoObject, MatchError, Params, Router, RouterError, RouterGen, RouterMapErr};
}

#[cfg(feature = "router")]
pub use router_priv::{Router, RouterError};
