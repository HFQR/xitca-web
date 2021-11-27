pub(crate) mod function;

use core::{future::Future, marker::PhantomData};

use crate::factory::ServiceFactory;
use crate::service::Service;

pub trait Transform<S, Req>: Clone {
    /// Responses produced by the service.
    type Response;

    /// Errors produced by the service.
    type Error;

    /// The `TransformService` value created by this factory
    type Transform: Service<Req, Response = Self::Response, Error = Self::Error>;

    /// Errors produced while building a transform service.
    type InitError;

    /// The future response value.
    type Future: Future<Output = Result<Self::Transform, Self::InitError>>;

    /// Creates and returns a new Transform component, asynchronously
    fn new_transform(&self, service: S) -> Self::Future;
}

pub struct TransformFactory<F, Req, T>
where
    F: ServiceFactory<Req>,
    T: Transform<F::Service, Req>,
{
    factory: F,
    transform: T,
    _req: PhantomData<Req>,
}

impl<F, Req, T> Clone for TransformFactory<F, Req, T>
where
    F: ServiceFactory<Req> + Clone,
    T: Transform<F::Service, Req>,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            transform: self.transform.clone(),
            _req: PhantomData,
        }
    }
}

impl<F, Req, T> TransformFactory<F, Req, T>
where
    F: ServiceFactory<Req>,
    T: Transform<F::Service, Req>,
{
    pub fn new(factory: F, transform: T) -> Self {
        Self {
            factory,
            transform,
            _req: PhantomData,
        }
    }
}

impl<F, Req, T> ServiceFactory<Req> for TransformFactory<F, Req, T>
where
    F: ServiceFactory<Req>,
    T: Transform<F::Service, Req>,
    F::InitError: From<T::InitError>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Config = F::Config;
    type Service = T::Transform;
    type InitError = F::InitError;
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);
        let transform = self.transform.clone();
        async move {
            let service = service.await?;
            let transform = transform.new_transform(service).await?;
            Ok(transform)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use core::{
        future::{ready, Ready},
        time::Duration,
    };

    // pseudo-doctest for Transform trait
    #[derive(Clone)]
    struct TimeoutTransform {
        timeout: Duration,
    }

    // pseudo-doctest for Transform trait
    impl<S, Req> Transform<S, Req> for TimeoutTransform
    where
        S: Service<Req>,
    {
        type Response = S::Response;
        type Error = S::Error;
        type Transform = Timeout<S>;
        type InitError = S::Error;
        type Future = impl Future<Output = Result<Self::Transform, Self::InitError>>;

        fn new_transform(&self, service: S) -> Self::Future {
            let service = Timeout {
                service,
                _timeout: self.timeout,
            };

            async { Ok(service) }
        }
    }

    // pseudo-doctest for Transform trait
    struct Timeout<S> {
        service: S,
        _timeout: Duration,
    }

    // pseudo-doctest for Transform trait
    impl<S, Req> Service<Req> for Timeout<S>
    where
        S: Service<Req>,
    {
        type Response = S::Response;
        type Error = S::Error;
        type Ready<'f>
        where
            Self: 'f,
        = Ready<Result<(), Self::Error>>;
        type Future<'f>
        where
            Self: 'f,
        = S::Future<'f>;

        fn ready(&self) -> Self::Ready<'_> {
            ready(Ok(()))
        }

        fn call(&self, req: Req) -> Self::Future<'_> {
            self.service.call(req)
        }
    }
}
