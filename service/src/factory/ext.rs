use core::future::Future;

use alloc::boxed::Box;

use crate::transform::{function::TransformFunctionFactory, Transform, TransformFactory};

use super::{map::MapServiceFactory, map_err::MapErrServiceFactory, ServiceFactory, ServiceFactoryObject};

pub trait ServiceFactoryExt<Req>: ServiceFactory<Req> {
    fn map<F, Res>(self, mapper: F) -> MapServiceFactory<Self, Req, F, Res>
    where
        F: Fn(Result<Self::Response, Self::Error>) -> Result<Res, Self::Error> + Clone,
        Self: Sized,
    {
        MapServiceFactory::new(self, mapper)
    }

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

    fn transform_fn<T, Fut>(self, transform: T) -> TransformFunctionFactory<Self, T>
    where
        T: for<'s> Fn(&'s Self::Service, Req) -> Fut + Clone,
        Fut: Future,
        Self: Sized,
    {
        TransformFunctionFactory::new(self, transform)
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

    async fn index(s: &'static str) -> Result<&'static str, ()> {
        Ok(s)
    }

    #[tokio::test]
    async fn service_object() {
        let factory = fn_service(index).transform(DummyMiddleware).into_object();

        let service = factory.new_service(()).await.unwrap();

        let res = service.call("996").await.unwrap();
        assert_eq!(res, "996");
    }

    #[tokio::test]
    async fn map() {
        let factory = fn_service(index)
            .map(|res| {
                let str = res?;
                assert_eq!(str, "996");
                Err::<(), _>(())
            })
            .map_err(|_| "251");

        let service = factory.new_service(()).await.unwrap();

        let err = service.call("996").await.err().unwrap();
        assert_eq!(err, "251");
    }

    #[tokio::test]
    async fn transform_fn() {
        let factory = fn_service(index).transform_fn(|service, req| {
            let service = service.clone();
            async move {
                let res = service.call(req).await?;
                assert_eq!(res, "996");
                Ok::<&'static str, ()>("251")
            }
        });

        let service = factory.new_service(()).await.unwrap();

        let res = service.call("996").await.ok().unwrap();
        assert_eq!(res, "251");
    }
}
