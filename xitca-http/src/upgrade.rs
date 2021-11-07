//! Default upgrade handler. It close the connection.

use std::future::{ready, Future, Ready};

use xitca_service::{Service, ServiceFactory};

pub struct UpgradeHandler;

impl Default for UpgradeHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl UpgradeHandler {
    pub const fn new() -> Self {
        Self
    }
}

impl<Req> ServiceFactory<Req> for UpgradeHandler {
    type Response = ();
    type Error = ();
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async { unimplemented!("Default UpgradeHandler must not be used") }
    }
}

impl<Req> Service<Req> for UpgradeHandler {
    type Response = ();
    type Error = ();
    type Ready<'f> = Ready<Result<(), Self::Error>>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        ready(Ok(()))
    }

    fn call(&self, _req: Req) -> Self::Future<'_> {
        async { unimplemented!("Default UpgradeHandler must not be used") }
    }
}
