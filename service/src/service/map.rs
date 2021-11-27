use core::{future::Future, marker::PhantomData};

use super::Service;

/// Service for the `map` combinator, changing the type of a service's response.
///
/// This is created by the `ServiceExt::map` method.
pub struct Map<S, Req, F, Res> {
    service: S,
    mapper: F,
    _phantom: PhantomData<(Req, Res)>,
}

impl<S, Req, F, Res> Map<S, Req, F, Res> {
    /// Create new `Map` combinator
    pub(crate) fn new(service: S, mapper: F) -> Self
    where
        S: Service<Req>,
        F: Fn(Result<S::Response, S::Error>) -> Result<Res, S::Error>,
    {
        Self {
            service,
            mapper,
            _phantom: PhantomData,
        }
    }
}

impl<S, Req, F, Res> Clone for Map<S, Req, F, Res>
where
    S: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Map {
            service: self.service.clone(),
            mapper: self.mapper.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<S, Req, F, Res> Service<Req> for Map<S, Req, F, Res>
where
    S: Service<Req>,
    F: Fn(Result<S::Response, S::Error>) -> Result<Res, S::Error>,
{
    type Response = Res;
    type Error = S::Error;
    type Ready<'f>
    where
        Self: 'f,
    = S::Ready<'f>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        self.service.ready()
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let res = self.service.call(req).await;
            (self.mapper)(res)
        }
    }
}
