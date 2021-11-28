use core::marker::PhantomData;

use crate::factory::pipeline::marker;

use super::Service;

/// Service for the [crate::factory::pipeline::PipelineServiceFactory]
///
/// [crate::factory::pipeline::marker] is used as `M` type for specialization
/// [Service] trait impl of different usage.
pub struct PipelineService<S, S1, M = ()> {
    pub(super) service: S,
    pub(super) service2: S1,
    _marker: PhantomData<M>,
}

impl<S, S1, M> Clone for PipelineService<S, S1, M>
where
    S: Clone,
    S1: Clone,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            service2: self.service2.clone(),
            _marker: PhantomData,
        }
    }
}

impl<S, S1> PipelineService<S, S1> {
    /// Create new `Map` combinator
    pub(crate) fn new_map<Req, Res>(service: S, service2: S1) -> PipelineService<S, S1, marker::Map>
    where
        S: Service<Req>,
        S1: Fn(Result<S::Response, S::Error>) -> Result<Res, S::Error>,
    {
        PipelineService {
            service,
            service2,
            _marker: PhantomData,
        }
    }

    /// Create new `MapErr` combinator
    pub(crate) fn new_map_err<Req, E>(service: S, service2: S1) -> PipelineService<S, S1, marker::MapErr>
    where
        S: Service<Req>,
        S1: Fn(S::Error) -> E,
    {
        PipelineService {
            service,
            service2,
            _marker: PhantomData,
        }
    }

    /// Create new `Then` combinator
    pub(crate) fn new_then<Req>(service: S, service2: S1) -> PipelineService<S, S1, marker::Then>
    where
        S: Service<Req>,
        S1: Service<Result<S::Response, S::Error>>,
    {
        PipelineService {
            service,
            service2,
            _marker: PhantomData,
        }
    }
}
