use core::{future::Future, marker::PhantomData};

use super::ServiceFactory;

use crate::service::map_err::MapErr;

/// Factory for the `map_err` combinator, changing the type of a new
/// service's error.
///
/// This is created by the `NewServiceExt::map_err` method.
pub struct MapErrServiceFactory<SF, Req, F, E>
where
    SF: ServiceFactory<Req>,
    F: Fn(SF::Error) -> E + Clone,
{
    factory: SF,
    mapper: F,
    _err: PhantomData<(Req, E)>,
}

impl<SF, Req, F, E> MapErrServiceFactory<SF, Req, F, E>
where
    SF: ServiceFactory<Req>,
    F: Fn(SF::Error) -> E + Clone,
{
    /// Create new `MapErr` new service instance
    pub(super) fn new(factory: SF, mapper: F) -> Self {
        Self {
            factory,
            mapper,
            _err: PhantomData,
        }
    }
}

impl<SF, Req, F, E> Clone for MapErrServiceFactory<SF, Req, F, E>
where
    SF: ServiceFactory<Req> + Clone,
    F: Fn(SF::Error) -> E + Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            mapper: self.mapper.clone(),
            _err: PhantomData,
        }
    }
}

impl<SF, Req, F, E> ServiceFactory<Req> for MapErrServiceFactory<SF, Req, F, E>
where
    SF: ServiceFactory<Req>,
    F: Fn(SF::Error) -> E + Clone,
{
    type Response = SF::Response;
    type Error = E;

    type Config = SF::Config;
    type Service = MapErr<SF::Service, Req, F, E>;
    type InitError = SF::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: SF::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let mapper = self.mapper.clone();
        async move {
            let service = service.await?;
            Ok(MapErr::new(service, mapper))
        }
    }
}
