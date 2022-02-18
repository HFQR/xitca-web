use core::future::Future;

use super::{
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory,
};

/// Type alias for specialized [PipelineServiceFactory].
pub type TransformFactory<F, T> = PipelineServiceFactory<F, T, marker::Transform>;

impl<F, Req, Arg, T> ServiceFactory<Req, Arg> for PipelineServiceFactory<F, T, marker::Transform>
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
        let service = self.factory.new_service(arg);
        let middleware = self.factory2.clone();
        async move {
            let service = service.await?;
            let middleware = middleware.new_service(service).await?;
            Ok(middleware)
        }
    }
}
