use core::{future::Future, marker::PhantomData};

use crate::service::map::Map;

use super::ServiceFactory;

/// `MapNewService` new service combinator
pub struct MapServiceFactory<SF, Req, F, Res> {
    factory: SF,
    mapper: F,
    _phantom: PhantomData<(Req, Res)>,
}

impl<SF, Req, F, Res> MapServiceFactory<SF, Req, F, Res> {
    /// Create new `Map` new service instance
    pub(super) fn new(factory: SF, mapper: F) -> Self
    where
        SF: ServiceFactory<Req>,
        F: Fn(Result<SF::Response, SF::Error>) -> Result<Res, SF::Error>,
    {
        Self {
            factory,
            mapper,
            _phantom: PhantomData,
        }
    }
}

impl<SF, Req, F, Res> Clone for MapServiceFactory<SF, Req, F, Res>
where
    SF: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            mapper: self.mapper.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<SF, Req, F, Res> ServiceFactory<Req> for MapServiceFactory<SF, Req, F, Res>
where
    SF: ServiceFactory<Req>,
    F: Fn(Result<SF::Response, SF::Error>) -> Result<Res, SF::Error> + Clone,
{
    type Response = Res;
    type Error = SF::Error;
    type Config = SF::Config;
    type Service = Map<SF::Service, Req, F, Res>;
    type InitError = SF::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: SF::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let mapper = self.mapper.clone();

        async move {
            let service = service.await?;
            Ok(Map::new(service, mapper))
        }
    }
}
