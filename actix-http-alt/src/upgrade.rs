//! Default upgrade handler. It close the connection.

use std::{
    future::Future,
    task::{Context, Poll},
};

use actix_service_alt::{Service, ServiceFactory};

pub struct UpgradeHandler;

impl Default for UpgradeHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl UpgradeHandler {
    pub fn new() -> Self {
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
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'c>(&'c self, _req: Req) -> Self::Future<'c> {
        async { unimplemented!("Default UpgradeHandler must not be used") }
    }
}
