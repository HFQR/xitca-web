//! A middleware construct self from `Option<T: BuildService<S: Service<Req>>>`.

use core::future::Future;

use crate::{build::BuildService, pipeline::PipelineE};

impl<T, S> BuildService<S> for Option<T>
where
    T: BuildService<S>,
{
    type Service = PipelineE<S, T::Service>;
    type Error = T::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, service: S) -> Self::Future {
        let pipe = match self {
            None => PipelineE::First(service),
            Some(ref t) => PipelineE::Second(t.build(service)),
        };

        async {
            match pipe {
                PipelineE::First(service) => Ok(PipelineE::First(service)),
                PipelineE::Second(fut) => fut.await.map(PipelineE::Second),
            }
        }
    }
}
