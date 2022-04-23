use core::future::Future;

use crate::pipeline::{marker::Enclosed, PipelineT};

use super::ServiceFactory;

impl<F, Req, Arg, T> ServiceFactory<Req, Arg> for PipelineT<F, T, Enclosed>
where
    F: ServiceFactory<Req, Arg>,
    T: ServiceFactory<Req, F::Service> + Clone,
    T::Error: From<F::Error>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Service = T::Service;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.first.new_service(arg);
        let middleware = self.second.clone();
        async move {
            let service = service.await?;
            let middleware = middleware.new_service(service).await?;
            Ok(middleware)
        }
    }
}
