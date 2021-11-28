use core::marker::PhantomData;

use super::ServiceFactory;

/// Constructor type for two step ServiceFactory where field are called
/// with top down order.
pub struct PipelineServiceFactory<SF, SF1, M = ()> {
    pub(super) factory: SF,
    pub(super) factory2: SF1,
    _marker: PhantomData<M>,
}

/// Marker types for different variant of [PipelineServiceFactory] as [crate::service::pipeline::PipelineService]
pub(crate) mod marker {
    pub struct Map;
    pub struct MapErr;
    pub struct Then;
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

impl<SF, SF1> PipelineServiceFactory<SF, SF1> {
    /// Create new `Map` new service instance
    pub(super) fn new_map<Req, Res>(factory: SF, factory2: SF1) -> PipelineServiceFactory<SF, SF1, marker::Map>
    where
        SF: ServiceFactory<Req>,
        SF1: Fn(Result<SF::Response, SF::Error>) -> Result<Res, SF::Error>,
    {
        PipelineServiceFactory {
            factory,
            factory2,
            _marker: PhantomData,
        }
    }

    /// Create new `MapErr` new service instance
    pub(super) fn new_map_err<Req, E>(factory: SF, factory2: SF1) -> PipelineServiceFactory<SF, SF1, marker::MapErr>
    where
        SF: ServiceFactory<Req>,
        SF1: Fn(SF::Error) -> E + Clone,
    {
        PipelineServiceFactory {
            factory,
            factory2,
            _marker: PhantomData,
        }
    }

    /// Create new `Then` new service instance
    pub(super) fn new_then<Req>(factory: SF, factory2: SF1) -> PipelineServiceFactory<SF, SF1, marker::Then>
    where
        SF: ServiceFactory<Req>,
        SF1: ServiceFactory<Result<SF::Response, SF::Error>>,
    {
        PipelineServiceFactory {
            factory,
            factory2,
            _marker: PhantomData,
        }
    }
}
