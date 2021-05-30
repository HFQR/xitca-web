use super::ServiceFactory;

use crate::service::Service;
use crate::transform::{Transform, TransformFactory};

pub trait ServiceFactoryExt<Req>: ServiceFactory<Req> {
    fn transform<T, S>(self, transform: T) -> TransformFactory<Self, S, Req, T>
    where
        T: Transform<S, Req>,
        S: Service<Req>,
        Self: ServiceFactory<Req, Service = S> + Sized,
    {
        TransformFactory::new(self, transform)
    }
}

impl<F, Req> ServiceFactoryExt<Req> for F where F: ServiceFactory<Req> {}
