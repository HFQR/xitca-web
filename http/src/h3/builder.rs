use xitca_service::Service;

use super::service::{H3Service, H3ServiceV2};

/// Http/3 Builder type.
/// Take in generic types of ServiceFactory for `quinn`.
pub struct H3ServiceBuilder;

impl Default for H3ServiceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl H3ServiceBuilder {
    /// Construct a new Service Builder with given service factory.
    pub fn new() -> Self {
        H3ServiceBuilder
    }
}

impl<S, E> Service<Result<S, E>> for H3ServiceBuilder {
    type Response = H3Service<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(H3Service::new)
    }
}

/// Http/3 Builder type backed by the homebrew dispatcher (v2).
///
/// Mirrors [`H3ServiceBuilder`] but yields a [`H3ServiceV2`] that drives
/// the in-tree HTTP/3 implementation rather than the `h3` crate.
pub struct H3ServiceBuilderV2;

impl Default for H3ServiceBuilderV2 {
    fn default() -> Self {
        Self::new()
    }
}

impl H3ServiceBuilderV2 {
    /// Construct a new Service Builder for the homebrew dispatcher.
    pub fn new() -> Self {
        H3ServiceBuilderV2
    }
}

impl<S, E> Service<Result<S, E>> for H3ServiceBuilderV2 {
    type Response = H3ServiceV2<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(H3ServiceV2::new)
    }
}
