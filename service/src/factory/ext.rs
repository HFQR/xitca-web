use alloc::boxed::Box;

use crate::transform::{Transform, TransformFactory};

use super::{map_err::MapErrServiceFactory, ServiceFactory, ServiceFactoryObject};

pub trait ServiceFactoryExt<Req>: ServiceFactory<Req> {
    fn map_err<F, E>(self, err: F) -> MapErrServiceFactory<Self, Req, F, E>
    where
        F: Fn(Self::Error) -> E + Clone,
        Self: Sized,
    {
        MapErrServiceFactory::new(self, err)
    }

    fn transform<T>(self, transform: T) -> TransformFactory<Self, Req, T>
    where
        T: Transform<Self::Service, Req>,
        Self: ServiceFactory<Req> + Sized,
    {
        TransformFactory::new(self, transform)
    }

    fn into_object(self) -> ServiceFactoryObject<Req, Self::Response, Self::Error, Self::Config, Self::InitError>
    where
        Self: Sized + 'static,
        Self::Service: 'static,
        Self::Future: 'static,
        Req: 'static,
    {
        Box::new(self)
    }
}

impl<F, Req> ServiceFactoryExt<Req> for F where F: ServiceFactory<Req> {}

#[cfg(test)]
mod test {
    use super::*;

    use core::future::Future;

    use crate::{fn_service, Service};

    #[derive(Clone)]
    struct DummyMiddleware;

    struct DummyMiddlewareService<S>(S);

    impl<S, Req> Transform<S, Req> for DummyMiddleware
    where
        S: Service<Req>,
    {
        type Response = S::Response;

        type Error = S::Error;

        type Transform = DummyMiddlewareService<S>;

        type InitError = ();

        type Future = impl Future<Output = Result<Self::Transform, Self::InitError>>;

        fn new_transform(&self, service: S) -> Self::Future {
            async { Ok(DummyMiddlewareService(service)) }
        }
    }

    impl<S, Req> Service<Req> for DummyMiddlewareService<S>
    where
        S: Service<Req>,
    {
        type Response = S::Response;

        type Error = S::Error;

        type Ready<'f>
        where
            S: 'f,
        = impl Future<Output = Result<(), Self::Error>>;

        type Future<'f>
        where
            S: 'f,
        = impl Future<Output = Result<Self::Response, Self::Error>>;

        fn ready(&self) -> Self::Ready<'_> {
            async move { self.0.ready().await }
        }

        fn call(&self, req: Req) -> Self::Future<'_> {
            async move { self.0.call(req).await }
        }
    }

    #[tokio::test]
    async fn service_object() {
        async fn index(s: &'static str) -> Result<&'static str, ()> {
            Ok(s)
        }

        let factory = fn_service(index).transform(DummyMiddleware).into_object();

        let service = factory.new_service(()).await.unwrap();

        let res = service.call("996").await.unwrap();
        assert_eq!(res, "996");
    }
}
