use core::{future::Future, marker::PhantomData};

use super::Service;

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
    pub(crate) fn new(service: S, mapper: F) -> Self
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
