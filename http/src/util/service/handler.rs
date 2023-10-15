use core::{
    convert::Infallible,
    future::Future,
    future::{ready, Ready},
    marker::PhantomData,
};

use xitca_service::{fn_build, pipeline::PipelineE, AsyncClosure, FnService, Service};

/// A service factory shortcut offering given async function ability to use [FromRequest] to destruct and transform `Service<Req>`'s
/// `Req` type and receive them as function argument.
///
/// Given async function's return type must impl [Responder] trait for transforming arbitrary return type to `Service::Future`'s
/// output type.
pub fn handler_service<Arg, F, T, O>(
    func: F,
) -> FnService<impl Fn(Arg) -> Ready<Result<HandlerService<F, T, O>, Infallible>>>
where
    F: AsyncClosure<T> + Clone,
{
    fn_build(move |_| ready(Ok(HandlerService::new(func.clone()))))
}

pub struct HandlerService<F, T, O> {
    func: F,
    _p: PhantomData<(T, O)>,
}

impl<F, T, O> HandlerService<F, T, O> {
    pub const fn new(func: F) -> Self {
        Self { func, _p: PhantomData }
    }
}

impl<F, Req, T, O> Service<Req> for HandlerService<F, T, O>
where
    // for borrowed extractors, `T` is the `'static` version of the extractors
    T: FromRequest<Req>,
    // just to assist type inference to pinpoint `T`
    F: AsyncClosure<T>,
    F: for<'a> AsyncClosure<T::Type<'a>, Output = O>,
    O: Responder<Req>,
{
    type Response = O::Output;
    type Error = T::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        let extract = T::from_request(&req).await?;
        let res = self.func.call(extract).await;
        Ok(res.respond_to(req).await)
    }
}

/// Extract type from Req and receive them with function passed to [handler_service].
///
/// `'a` is the lifetime of the extracted type.
///
/// When `Req` is also a borrowed type, the lifetimes of `Req` type and of the extracted type
/// should be kept separate. See the example below.
///
/// # Examples
/// ```
/// # use std::future::Future;
/// # use xitca_http::util::service::handler::FromRequest;
/// struct MyExtractor<'a>(&'a str);
///
/// impl<'r> FromRequest<&'r String> for MyExtractor<'_> {
///     type Type<'b> = MyExtractor<'b>;
///     type Error = ();
///
///     async fn from_request<'a>(req: &'a &'r String) -> Result<Self::Type<'a>, Self::Error> {
///         Ok(MyExtractor(req))
///     }
/// }
/// ```
pub trait FromRequest<Req>: Sized {
    // Used to construct the type for any lifetime 'b.
    type Type<'b>;
    type Error;

    fn from_request(req: &Req) -> impl Future<Output = Result<Self::Type<'_>, Self::Error>>;
}

macro_rules! from_req_impl {
    ($req0: ident, $($req: ident,)*) => {
        impl<Req, $req0, $($req,)*> FromRequest<Req> for ($req0, $($req,)*)
        where
            $req0: FromRequest<Req>,
            $(
                $req: FromRequest<Req>,
                $req0::Error: From<$req::Error>,
            )*
        {
            type Type<'r> = ($req0::Type<'r>, $($req::Type<'r>,)*);
            type Error = $req0::Error;

            #[inline]
            async fn from_request(req: &Req) -> Result<Self::Type<'_>, Self::Error> {
                Ok((
                    $req0::from_request(req).await?,
                    $($req::from_request(req).await?,)*
                ))
            }
        }
    }
}

from_req_impl! { A, }
from_req_impl! { A, B, }
from_req_impl! { A, B, C, }
from_req_impl! { A, B, C, D, }
from_req_impl! { A, B, C, D, E, }
from_req_impl! { A, B, C, D, E, F, }
from_req_impl! { A, B, C, D, E, F, G, }
from_req_impl! { A, B, C, D, E, F, G, H, }
from_req_impl! { A, B, C, D, E, F, G, H, I, }

/// Make Response with reference of Req.
/// The Output type is what returns from [handler_service] function.
pub trait Responder<Req> {
    type Output;

    fn respond_to(self, req: Req) -> impl Future<Output = Self::Output>;
}

impl<R, T, E> Responder<R> for Result<T, E>
where
    T: Responder<R>,
{
    type Output = Result<T::Output, E>;

    #[inline]
    async fn respond_to(self, req: R) -> Self::Output {
        Ok(self?.respond_to(req).await)
    }
}

impl<R, F, S> Responder<R> for PipelineE<F, S>
where
    F: Responder<R>,
    S: Responder<R, Output = F::Output>,
{
    type Output = F::Output;

    #[inline]
    async fn respond_to(self, req: R) -> Self::Output {
        match self {
            Self::First(f) => f.respond_to(req).await,
            Self::Second(s) => s.respond_to(req).await,
        }
    }
}

#[cfg(test)]
mod test {
    use std::convert::Infallible;

    use xitca_service::{Service, ServiceExt};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::http::{Request, RequestExt, Response, StatusCode};

    use super::*;

    async fn handler(e1: String, e2: u32, (_, e3): (&Request<RequestExt<()>>, u64)) -> StatusCode {
        assert_eq!(e1, "996");
        assert_eq!(e2, 996);
        assert_eq!(e3, 996);

        StatusCode::MULTI_STATUS
    }

    impl Responder<Request<RequestExt<()>>> for StatusCode {
        type Output = Response<()>;

        async fn respond_to(self, _: Request<RequestExt<()>>) -> Self::Output {
            let mut res = Response::new(());
            *res.status_mut() = self;
            res
        }
    }

    impl FromRequest<Request<RequestExt<()>>> for String {
        type Type<'f> = Self;
        type Error = Infallible;

        async fn from_request(_: &Request<RequestExt<()>>) -> Result<Self::Type<'_>, Self::Error> {
            Ok(String::from("996"))
        }
    }

    impl FromRequest<Request<RequestExt<()>>> for u32 {
        type Type<'f> = Self;
        type Error = Infallible;

        async fn from_request(_: &Request<RequestExt<()>>) -> Result<Self::Type<'_>, Self::Error> {
            Ok(996)
        }
    }

    impl FromRequest<Request<RequestExt<()>>> for u64 {
        type Type<'f> = Self;
        type Error = Infallible;

        async fn from_request(_: &Request<RequestExt<()>>) -> Result<Self::Type<'_>, Self::Error> {
            Ok(996)
        }
    }

    impl FromRequest<Request<RequestExt<()>>> for &Request<RequestExt<()>> {
        type Type<'f> = &'f Request<RequestExt<()>>;
        type Error = Infallible;

        async fn from_request(req: &Request<RequestExt<()>>) -> Result<Self::Type<'_>, Self::Error> {
            Ok(req)
        }
    }

    #[test]
    fn concurrent_extract_with_enclosed_fn() {
        async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
        where
            S: Service<Req>,
        {
            service.call(req).await
        }

        let res = handler_service(handler)
            .enclosed_fn(enclosed)
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status(), StatusCode::MULTI_STATUS);
    }

    #[cfg(feature = "router")]
    #[test]
    fn handler_in_router() {
        use crate::util::service::{route::get, Router};

        let res = Router::new()
            .insert("/", get(handler_service(handler)))
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status(), StatusCode::MULTI_STATUS);
    }
}
