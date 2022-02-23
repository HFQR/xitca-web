use core::future::Future;

use alloc::boxed::Box;

use super::{
    boxed::BoxedServiceFactory,
    pipeline::{marker, PipelineServiceFactory},
    ServiceFactory, ServiceFactoryObject,
};

pub trait ServiceFactoryExt<Req, Arg>: ServiceFactory<Req, Arg> {
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

    /// Box `<Self as ServiceFactory<_>>::Future` to reduce it's stack size.
    ///
    /// *. This combinator does not box `Self` or `Self::Service`.
    fn boxed_future(self) -> BoxedServiceFactory<Self>
    where
        Self: Sized,
    {
        BoxedServiceFactory::new(self)
    }

    /// Chain another service factory who's service takes `Self`'s `Service::Response` output as
    /// `Service::Request`.
    ///
    /// *. Unlike `then` combinator both `F` and `Self`'s readiness are checked beforehand.
    fn and_then<F>(self, factory: F) -> PipelineServiceFactory<Self, F, marker::AndThen>
    where
        F: ServiceFactory<Self::Response, Arg>,
        Self: Sized,
    {
        PipelineServiceFactory::new(self, factory)
    }

    fn enclosed<T>(self, transform: T) -> PipelineServiceFactory<Self, T, marker::Transform>
    where
        T: ServiceFactory<Req, Self::Service> + Clone,
        Self: ServiceFactory<Req, Arg> + Sized,
    {
        PipelineServiceFactory::new(self, transform)
    }

    fn enclosed_fn<T, Fut>(self, transform: T) -> PipelineServiceFactory<Self, T, marker::TransformFn>
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
    fn into_object(self) -> ServiceFactoryObject<Req, Arg, Self::Response, Self::Error>
    where
        Self: Sized + 'static,
        Self::Service: Clone + 'static,
        Self::Future: 'static,
        Req: 'static,
    {
        Box::new(self)
    }
}

impl<F, Req, Arg> ServiceFactoryExt<Req, Arg> for F where F: ServiceFactory<Req, Arg> {}

#[cfg(test)]
mod test {
    use super::*;

    use core::future::Future;

    use crate::{fn_service, Service};

    #[derive(Clone)]
    struct DummyMiddleware;

    #[derive(Clone)]
    struct DummyMiddlewareService<S: Clone>(S);

    impl<S, Req> ServiceFactory<Req, S> for DummyMiddleware
    where
        S: Service<Req> + Clone,
    {
        type Response = S::Response;
        type Error = S::Error;
        type Service = DummyMiddlewareService<S>;
        type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

        fn new_service(&self, service: S) -> Self::Future {
            async { Ok(DummyMiddlewareService(service)) }
        }
    }

    impl<S, Req> Service<Req> for DummyMiddlewareService<S>
    where
        S: Service<Req> + Clone,
    {
        type Response = S::Response;
        type Error = S::Error;
        type Future<'f>
        where
            S: 'f,
        = impl Future<Output = Result<Self::Response, Self::Error>>;

        fn call(&self, req: Req) -> Self::Future<'_> {
            async move { self.0.call(req).await }
        }
    }

    async fn index(s: &'static str) -> Result<&'static str, ()> {
        Ok(s)
    }

    #[tokio::test]
    async fn service_object() {
        let factory = fn_service(index).enclosed(DummyMiddleware).into_object();

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
        let factory = fn_service(index).enclosed_fn(|service, req| async move {
            let res = service.call(req).await?;
            assert_eq!(res, "996");
            Ok::<&'static str, ()>("251")
        });

        let service = factory.new_service(()).await.unwrap();

        let res = service.call("996").await.ok().unwrap();
        assert_eq!(res, "251");
    }
}
