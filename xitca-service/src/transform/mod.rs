use core::{future::Future, marker::PhantomData};

use alloc::rc::Rc;

use crate::factory::ServiceFactory;
use crate::service::Service;

pub trait Transform<S, Req> {
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

pub struct TransformFactory<F, S, Req, T>
where
    F: ServiceFactory<Req>,
    T: Transform<S, Req>,
{
    factory: F,
    transform: Rc<T>,
    _req: PhantomData<(S, Req)>,
}

impl<F, S, Req, T> TransformFactory<F, S, Req, T>
where
    F: ServiceFactory<Req>,
    T: Transform<S, Req>,
{
    pub fn new(factory: F, transform: T) -> Self {
        Self {
            factory,
            transform: Rc::new(transform),
            _req: PhantomData,
        }
    }
}

impl<F, S, Req, T> ServiceFactory<Req> for TransformFactory<F, S, Req, T>
where
    F: ServiceFactory<Req, Service = S>,
    S: Service<Req>,
    T: Transform<S, Req>,
    // T::InitError: From<F::InitError>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Config = F::Config;
    type Service = T::Transform;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let service = self.factory.new_service(cfg);

        let transform = self.transform.clone();

        async move {
            let service = service.await.ok().unwrap();
            let transform = transform.new_transform(service).await.ok().unwrap();

            Ok(transform)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use core::{
        task::{Context, Poll},
        time::Duration,
    };

    // pseudo-doctest for Transform trait
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
        type Future<'f> = S::Future<'f>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&self, req: Req) -> Self::Future<'_> {
            self.service.call(req)
        }
    }
}
