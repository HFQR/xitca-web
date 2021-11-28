use core::marker::PhantomData;

/// Service for the [crate::factory::pipeline::PipelineServiceFactory]
///
/// [crate::factory::pipeline::marker] is used as `M` type for specialization
/// [Service] trait impl of different usage.
pub struct PipelineService<S, S1, M = ()> {
    pub(crate) service: S,
    pub(crate) service2: S1,
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

impl<S, S1, M> PipelineService<S, S1, M> {
    pub(crate) fn new(service: S, service2: S1) -> PipelineService<S, S1, M> {
        PipelineService {
            service,
            service2,
            _marker: PhantomData,
        }
    }
}
