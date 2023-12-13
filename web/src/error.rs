//! web error types.

pub use xitca_http::{
    error::BodyError,
    util::service::{
        route::MethodNotAllowed,
        router::{MatchError, RouterError},
    },
};
