use core::future::Future;

use crate::{Service, ServiceFactory};

pub struct TransformFunctionFactory<SF, T> {
    factory: SF,
    transform: T,
}

impl<SF, T> TransformFunctionFactory<SF, T> {
    pub(crate) fn new(factory: SF, transform: T) -> Self {
        Self { factory, transform }
    }
}

impl<SF, T> Clone for TransformFunctionFactory<SF, T>
where
    SF: Clone,
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            transform: self.transform.clone(),
        }
    }
}

impl<SF, Req, T, Fut, Res, Err> ServiceFactory<Req> for TransformFunctionFactory<SF, T>
where
    SF: ServiceFactory<Req>,
    T: for<'s> Fn(&'s SF::Service, Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<SF::Error>,
{
    type Response = Res;
    type Error = Err;
    type Config = SF::Config;
    type Service = TransformFunction<SF::Service, T>;
    type InitError = SF::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let transform = self.transform.clone();

        async move {
            let service = service.await?;
            Ok(TransformFunction { service, transform })
        }
    }
}

pub struct TransformFunction<S, T> {
    service: S,
    transform: T,
}

impl<S, T> Clone for TransformFunction<S, T>
where
    S: Clone,
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            transform: self.transform.clone(),
        }
    }
}

impl<S, Req, T, Fut, Res, Err> Service<Req> for TransformFunction<S, T>
where
    S: Service<Req>,
    T: for<'s> Fn(&'s S, Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S::Error>,
{
    type Response = Res;
    type Error = Err;
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
        async move { Ok(self.service.ready().await?) }
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        (self.transform)(&self.service, req)
    }
}
