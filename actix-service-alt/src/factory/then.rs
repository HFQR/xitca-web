use core::future::Future;

use crate::service::then::ThenService;

use super::ServiceFactory;

pub struct ThenFactory<F1, F2> {
    f1: F1,
    f2: F2,
}

impl<F1, F2> ThenFactory<F1, F2> {
    pub fn new(f1: F1, f2: F2) -> Self {
        Self { f1, f2 }
    }
}

impl<F1, Req, F2> ServiceFactory<Req> for ThenFactory<F1, F2>
where
    F1: ServiceFactory<Req>,
    F1::Config: Clone,
    F2: ServiceFactory<F1::Response, Config = F1::Config>,

    F1::InitError: From<F2::InitError>,
    F1::Error: From<F2::Error>,
{
    type Response = F2::Response;
    type Error = F1::Error;
    type Config = F1::Config;
    type Service = ThenService<F1::Service, F2::Service>;
    type InitError = F1::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let s1 = self.f1.new_service(cfg.clone());
        let s2 = self.f2.new_service(cfg);
        async move {
            let s1 = s1.await?;
            let s2 = s2.await?;

            Ok(ThenService::new(s1, s2))
        }
    }
}
