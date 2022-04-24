use core::future::Future;

use crate::{
    async_closure::AsyncClosure,
    pipeline::{marker::EnclosedFn, PipelineT},
};

use super::ServiceFactory;

impl<SF, Req, Arg, T, Res, Err> ServiceFactory<Req, Arg> for PipelineT<SF, T, EnclosedFn>
where
    SF: ServiceFactory<Req, Arg>,
    T: for<'s> AsyncClosure<(&'s SF::Service, Req), Output = Result<Res, Err>> + Clone,
    Err: From<SF::Error>,
{
    type Response = Res;
    type Error = Err;
    type Service = PipelineT<SF::Service, T, EnclosedFn>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let service = self.first.new_service(arg);
        let transform = self.second.clone();
        async move {
            let service = service.await?;
            Ok(PipelineT::new(service, transform))
        }
    }
}
