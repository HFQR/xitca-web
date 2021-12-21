use core::future::Future;

use alloc::boxed::Box;

use crate::transform::Transform;

use super::{
    boxed::BoxedServiceFactory,
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory, ServiceFactoryObject, ServiceFactoryObjectTrait,
};

pub trait ServiceFactoryExt<Req>: ServiceFactory<Req> {
    fn map<F, Res>(self, mapper: F) -> PipelineServiceFactory<Self, F, marker::Map>
    where
        F: Fn(Result<Self::Response, Self::Error>) -> Result<Res, Self::Error> + Clone,
        Self: Sized,
    {
        PipelineServiceFactory::new(self, mapper)
    }

    fn map_err<F, E>(self, err: F) -> PipelineServiceFactory<Self, F, marker::MapErr>
    where
        F: Fn(Self::Error) -> E + Clone,
        Self: Sized,
    {
        PipelineServiceFactory::new(self, err)
    }

    fn map_init_err<F, E>(self, err: F) -> PipelineServiceFactory<Self, F, marker::MapInitErr>
    where
        F: Fn(Self::InitError) -> E + Clone,
        Self: Sized,
    {
        PipelineServiceFactory::new(self, err)
    }

    /// Box `<Self as ServiceFactory<_>>::Future` to reduce it's stack size.
    ///
    /// *. This combinator does not box `Self` or `Self::Service`.
    fn boxed_future(self) -> BoxedServiceFactory<Self>
    where
        Self: Sized,
    {
        BoxedServiceFactory::new(self)
    }

    /// Chain another service factory who's service takes `Self`'s `Service::Future` output as
    /// `Service::Request`.
    ///
    /// *. Only `F`'s readiness is checked beforehand.
    /// `Self::Service`'s readiness is checked inside `<F::Service as Service>::call`.
    /// This way the readiness error would be able to be handled by `F`.
    fn then<F>(self, factory: F) -> PipelineServiceFactory<Self, F, marker::Then>
    where
        F: ServiceFactory<Result<Self::Response, Self::Error>>,
        Self: Sized,
    {
        PipelineServiceFactory::new(self, factory)
    }

    /// Chain another service factory who's service takes `Self`'s `Service::Response` output as
    /// `Service::Request`.
    ///
    /// *. Unlike `then` combinator both `F` and `Self`'s readiness are checked beforehand.
    fn and_then<F>(self, factory: F) -> PipelineServiceFactory<Self, F, marker::AndThen>
    where
        F: ServiceFactory<Self::Response>,
        Self: Sized,
    {
        PipelineServiceFactory::new(self, factory)
    }

    fn transform<T>(self, transform: T) -> PipelineServiceFactory<Self, T, marker::Transform>
    where
        T: Transform<Self::Service, Req>,
        Self: ServiceFactory<Req> + Sized,
    {
        PipelineServiceFactory::new(self, transform)
    }

    fn transform_fn<T, Fut>(self, transform: T) -> PipelineServiceFactory<Self, T, marker::TransformFn>
    where
        T: Fn(Self::Service, Req) -> Fut + Clone,
        Fut: Future,
        Self: Sized,
        Self::Service: Clone,
    {
        PipelineServiceFactory::new(self, transform)
    }

    /// Box self and cast it to a trait object.
    ///
    /// This would erase `Self::Service` type and it's GAT nature.
    ///
    /// See [crate::service::ServiceObject] for detail.
    fn into_object(
        self,
    ) -> ServiceFactoryObject<Req, Self::Response, Self::Error, Self::Config, Self::InitError, Self::ServiceObj>
    where
        Self: Sized + 'static,
        Self: ServiceFactoryObjectTrait<Req, Self::Response, Self::Error, Self::Config, Self::InitError>,
    {
        ServiceFactoryObject(Box::new(self))
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

    #[derive(Clone)]
    struct DummyMiddlewareService<S: Clone>(S);

    impl<S, Req> Transform<S, Req> for DummyMiddleware
    where
        S: Service<Req> + Clone,
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
        S: Service<Req> + Clone,
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
        use crate::{Request, RequestSpecs};

        impl<'a, 'b> Request<'a, &'a &'b ()> for &'static str {
            type Type = &'static str;
        }

        impl RequestSpecs<&'static str> for &'static str {
            type Lifetime = &'static ();
            type Lifetimes = &'static &'static ();
        }

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
        let factory = fn_service(index).transform_fn(|service, req| async move {
            let res = service.call(req).await?;
            assert_eq!(res, "996");
            Ok::<&'static str, ()>("251")
        });

        let service = factory.new_service(()).await.unwrap();

        let res = service.call("996").await.ok().unwrap();
        assert_eq!(res, "251");
    }

    #[tokio::test]
    async fn then() {
        let factory = fn_service(index).then(fn_service(|res: Result<&'static str, ()>| async move {
            assert_eq!(res.ok().unwrap(), "996");
            Ok::<_, ()>("251")
        }));

        let service = factory.new_service(()).await.unwrap();

        let res = service.call("996").await.ok().unwrap();
        assert_eq!(res, "251");
    }

    #[tokio::test]
    async fn map_init_err() {
        let factory = fn_service(index)
            .map_init_err(|_| "init_err")
            .map_init_err(|_| ())
            .transform(DummyMiddleware);

        let service = factory.new_service(()).await.unwrap();

        let res = service.call("996").await.ok().unwrap();
        assert_eq!(res, "996");
    }
}
