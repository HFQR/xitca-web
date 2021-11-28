use core::marker::PhantomData;

/// Constructor type for two step ServiceFactory where field are called
/// with top down order.
pub struct PipelineServiceFactory<SF, SF1, M = ()> {
    pub(crate) factory: SF,
    pub(crate) factory2: SF1,
    _marker: PhantomData<M>,
}

/// Marker types for different variant of [PipelineServiceFactory] as [crate::service::pipeline::PipelineService]
pub(crate) mod marker {
    pub struct Map;
    pub struct MapErr;
    pub struct Then;
    pub struct Transform;
    pub struct TransformFn;
}

impl<SF, SF1, M> Clone for PipelineServiceFactory<SF, SF1, M>
where
    SF: Clone,
    SF1: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            factory2: self.factory2.clone(),
            _marker: PhantomData,
        }
    }
}

impl<SF, SF1, M> PipelineServiceFactory<SF, SF1, M> {
    pub(super) fn new(factory: SF, factory2: SF1) -> PipelineServiceFactory<SF, SF1, M> {
        PipelineServiceFactory {
            factory,
            factory2,
            _marker: PhantomData,
        }
    }
}
