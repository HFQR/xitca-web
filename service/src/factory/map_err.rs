use core::{future::Future, marker::PhantomData};

use super::{Service, ServiceFactory};

/// Service for the `map_err` combinator, changing the type of a service's error.
///
/// This is created by the `ServiceExt::map_err` method.
pub struct MapErr<S, Req, F, E> {
    service: S,
    mapper: F,
    _t: PhantomData<(Req, E)>,
}

impl<S, Req, F, E> MapErr<S, Req, F, E> {
    /// Create new `MapErr` combinator
    pub(super) fn new(service: S, mapper: F) -> Self
    where
        S: Service<Req>,
        F: Fn(S::Error) -> E,
    {
        Self {
            service,
            mapper,
            _t: PhantomData,
        }
    }
}

impl<S, Req, F, E> Clone for MapErr<S, Req, F, E>
where
    S: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        MapErr {
            service: self.service.clone(),
            mapper: self.mapper.clone(),
            _t: PhantomData,
        }
    }
}

impl<S, Req, F, E> Service<Req> for MapErr<S, Req, F, E>
where
    S: Service<Req>,
    F: Fn(S::Error) -> E,
{
    type Response = S::Response;
    type Error = E;
    type Ready<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        async move { self.service.ready().await.map_err(&self.mapper) }
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move { self.service.call(req).await.map_err(&self.mapper) }
    }
}

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
