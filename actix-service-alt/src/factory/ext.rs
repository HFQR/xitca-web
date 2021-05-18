use super::ServiceFactory;

use crate::transform::{Transform, TransformFactory};

pub trait ServiceFactoryExt<Req>: ServiceFactory<Req> {
    fn transform<T>(self, transform: T) -> TransformFactory<Self, Req, T>
    where
        T: Transform<Self::Service, Req>,
        Self: Sized,
    {
        TransformFactory::new(self, transform)
    }
}

impl<F, Req> ServiceFactoryExt<Req> for F where F: ServiceFactory<Req> {}
