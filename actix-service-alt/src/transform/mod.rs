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

pub struct TransformFactory<F, Req, T>
where
    F: ServiceFactory<Req>,
    T: Transform<F::Service, Req>,
{
    factory: F,
    transform: Rc<T>,
    _req: PhantomData<Req>,
}

impl<F, Req, T> TransformFactory<F, Req, T>
where
    F: ServiceFactory<Req>,
    T: Transform<F::Service, Req>,
{
    pub fn new(factory: F, transform: T) -> Self {
        Self {
            factory,
            transform: Rc::new(transform),
            _req: PhantomData,
        }
    }
}

impl<F, Req, T> ServiceFactory<Req> for TransformFactory<F, Req, T>
where
    F: ServiceFactory<Req>,
    T: Transform<F::Service, Req>,
    T::InitError: From<F::InitError>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Config = F::Config;
    type Service = T::Transform;
    type InitError = T::InitError;
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
