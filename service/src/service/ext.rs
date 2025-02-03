use crate::{
    middleware,
    pipeline::{PipelineT, marker},
};

use super::Service;

/// extend trait for [Service] providing combinator functionalities.
pub trait ServiceExt<Arg>: Service<Arg> {
    /// Enclose Self with given `T as Service<<Self as Service<_>>::Response>>`. In other word T
    /// would take Self's `Service::Response` type as it's generic argument of `Service<_>` impl.
    fn enclosed<T>(self, build: T) -> PipelineT<Self, T, marker::BuildEnclosed>
    where
        T: Service<Result<Self::Response, Self::Error>>,
        Self: Sized,
    {
        PipelineT::new(self, build)
    }

    /// Function version of [Self::enclosed] method.
    fn enclosed_fn<T, Req, O>(self, func: T) -> PipelineT<Self, middleware::AsyncFn<T>, marker::BuildEnclosed>
    where
        T: for<'s> AsyncFn(&'s Self::Response, Req) -> O + Clone,
        Self: Sized,
    {
        self.enclosed(middleware::AsyncFn(func))
    }

    /// Mutate `<<Self::Response as Service<Req>>::Future as Future>::Output` type with given
    /// closure.
    fn map<F, Res, ResMap>(self, mapper: F) -> PipelineT<Self, F, marker::BuildMap>
    where
        F: Fn(Res) -> ResMap + Clone,
        Self: Sized,
    {
        PipelineT::new(self, mapper)
    }

    /// Mutate `<Self::Response as Service<Req>>::Error` type with given closure.
    fn map_err<F, Err, ErrMap>(self, err: F) -> PipelineT<Self, F, marker::BuildMapErr>
    where
        F: Fn(Err) -> ErrMap + Clone,
        Self: Sized,
    {
        PipelineT::new(self, err)
    }

    /// Chain another service factory who's service takes `Self`'s `Service::Response` output as
    /// `Service::Request`.
    fn and_then<F>(self, factory: F) -> PipelineT<Self, F, marker::BuildAndThen>
    where
        F: Service<Arg>,
        Self: Sized,
    {
        PipelineT::new(self, factory)
    }
}

impl<S, Arg> ServiceExt<Arg> for S where S: Service<Arg> {}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::fn_service;

    use super::*;

    #[derive(Clone)]
    struct DummyMiddleware;

    struct DummyMiddlewareService<S>(S);

    impl<S, E> Service<Result<S, E>> for DummyMiddleware {
        type Response = DummyMiddlewareService<S>;
        type Error = E;

        async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
            res.map(DummyMiddlewareService)
        }
    }

    impl<S, Req> Service<Req> for DummyMiddlewareService<S>
    where
        S: Service<Req>,
    {
        type Response = S::Response;
        type Error = S::Error;

        async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
            self.0.call(req).await
        }
    }

    async fn index(s: &'static str) -> Result<&'static str, ()> {
        Ok(s)
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn service_object() {
        let service = fn_service(index)
            .enclosed(DummyMiddleware)
            .call(())
            .now_or_panic()
            .unwrap();

        let res = service.call("996").now_or_panic().unwrap();
        assert_eq!(res, "996");
    }

    #[test]
    fn map() {
        let service = fn_service(index).map(|_| "251").call(()).now_or_panic().unwrap();

        let err = service.call("996").now_or_panic().ok().unwrap();
        assert_eq!(err, "251");
    }

    #[test]
    fn map_err() {
        let service = fn_service(|_: &str| async { Err::<(), _>(()) })
            .map_err(|_| "251")
            .call(())
            .now_or_panic()
            .unwrap();

        let err = service.call("996").now_or_panic().err().unwrap();
        assert_eq!(err, "251");
    }

    #[test]
    fn enclosed_fn() {
        async fn enclosed<S>(service: &S, req: &'static str) -> Result<&'static str, ()>
        where
            S: Service<&'static str, Response = &'static str, Error = ()>,
        {
            let res = service.call(req).await?;
            assert_eq!(res, "996");
            Ok("251")
        }

        let res = fn_service(index)
            .enclosed_fn(enclosed)
            .enclosed_fn(async |service, req| service.call(req).await)
            .call(())
            .now_or_panic()
            .unwrap()
            .call("996")
            .now_or_panic()
            .ok()
            .unwrap();

        assert_eq!(res, "251");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn enclosed_opt() {
        let service = fn_service(index)
            .enclosed(Some(DummyMiddleware))
            .call(())
            .now_or_panic()
            .unwrap();

        let res = service.call("996").now_or_panic().unwrap();
        assert_eq!(res, "996");

        let service = fn_service(index)
            .enclosed(Option::<DummyMiddleware>::None)
            .call(())
            .now_or_panic()
            .unwrap();

        let res = service.call("996").now_or_panic().unwrap();
        assert_eq!(res, "996");
    }
}
