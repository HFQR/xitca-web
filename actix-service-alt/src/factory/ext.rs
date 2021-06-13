use crate::service::Service;
use crate::transform::{Transform, TransformFactory};

use super::then::ThenFactory;
use super::ServiceFactory;

pub trait ServiceFactoryExt<Req>: ServiceFactory<Req> {
    fn transform<T, S>(self, transform: T) -> TransformFactory<Self, S, Req, T>
    where
        T: Transform<S, Req>,
        S: Service<Req>,
        Self: ServiceFactory<Req, Service = S> + Sized,
    {
        TransformFactory::new(self, transform)
    }

    fn then<F>(self, other: F) -> ThenFactory<Self, F>
    where
        F: ServiceFactory<Self::Response>,
        Self: Sized,
    {
        ThenFactory::new(self, other)
    }
}

impl<F, Req> ServiceFactoryExt<Req> for F where F: ServiceFactory<Req> {}
