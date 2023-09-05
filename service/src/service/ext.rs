use crate::{
    async_closure::AsyncClosure,
    pipeline::{marker, PipelineT},
};

#[cfg(feature = "alloc")]
use crate::object::{DefaultObjectConstructor, IntoObject};

use super::Service;

pub trait ServiceExt<Arg>: Service<Arg> {
    /// Enclose Self with given `T as Service<<Self as Service<_>>::Response>>`.
    /// In other word T would take Self's Response type it's generic argument of `Service<_>`.
    fn enclosed<T>(self, build: T) -> PipelineT<Self, T, marker::BuildEnclosed>
    where
        T: Service<Self::Response>,
        Self: Sized,
    {
        PipelineT::new(self, build)
    }

    /// Function version of [Self::enclosed] method.
    fn enclosed_fn<T, Req>(self, func: T) -> PipelineT<Self, T, marker::BuildEnclosedFn>
    where
        T: for<'s> AsyncClosure<(&'s Self::Response, Req)> + Clone,
        Self: Sized,
    {
        PipelineT::new(self, func)
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

    #[cfg(feature = "alloc")]
    /// Box self and cast it to a trait object.
    ///
    /// This would erase `Self::Response` type and it's GAT nature.
    ///
    /// See [crate::object::DefaultObjectConstructor] for detail.
    fn into_object<Req>(self) -> <DefaultObjectConstructor as IntoObject<Self, Arg, Req>>::Object
    where
        Self: Sized,
        DefaultObjectConstructor: IntoObject<Self, Arg, Req>,
    {
        DefaultObjectConstructor::into_object(self)
    }
}

impl<S, Arg> ServiceExt<Arg> for S where S: Service<Arg> {}

#[cfg(test)]
mod test {
    use super::*;

    use core::{convert::Infallible, future::Future};

    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::fn_service;

    #[derive(Clone)]
    struct DummyMiddleware;

    #[derive(Clone)]
    struct DummyMiddlewareService<S>(S);

    impl<S: Clone> Service<S> for DummyMiddleware {
        type Response = DummyMiddlewareService<S>;
        type Error = Infallible;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f
        where
            Self: 'f,
            S: 'f;

        fn call<'s>(&'s self, service: S) -> Self::Future<'s>
        where
            S: 's,
        {
            async { Ok(DummyMiddlewareService(service)) }
        }
    }

    impl<S, Req> Service<Req> for DummyMiddlewareService<S>
    where
        S: Service<Req> + Clone,
    {
        type Response = S::Response;
        type Error = S::Error;
        type Future<'f> = S::Future<'f>
        where
            Self: 'f,
            Req: 'f;

        fn call<'s>(&'s self, req: Req) -> Self::Future<'s>
        where
            Req: 's,
        {
            self.0.call(req)
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
            .into_object()
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
            let res = service.call(req).now_or_panic()?;
            assert_eq!(res, "996");
            Ok("251")
        }

        let res = fn_service(index)
            .enclosed_fn(enclosed)
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
            .into_object()
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
