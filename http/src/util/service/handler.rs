#![allow(non_snake_case)]

use std::convert::Infallible;
use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use xitca_service::{BuildService, Service};

/// A service factory shortcut offering given async function ability to use [FromRequest] to destruct and transform `Service<Req>`'s
/// `Req` type and receive them as function argument.
///
/// Given async function's return type must impl [Responder] trait for transforming arbitrary return type to `Service::Future`'s
/// output type.
pub fn handler_service<F, T, O, Res, Err>(func: F) -> HandlerService<F, T, O, Res, Err> {
    HandlerService::new(func)
}

pub struct HandlerService<F, T, O, Res, Err> {
    func: F,
    _p: PhantomData<(T, O, Res, Err)>,
}

impl<F, T, O, Res, Err> HandlerService<F, T, O, Res, Err> {
    pub fn new(func: F) -> Self {
        Self { func, _p: PhantomData }
    }
}

impl<F, T, O, Res, Err> Clone for HandlerService<F, T, O, Res, Err>
where
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            func: self.func.clone(),
            _p: PhantomData,
        }
    }
}

impl<F, T, O, Res, Err> BuildService for HandlerService<F, T, O, Res, Err>
where
    F: Clone,
{
    type Service = Self;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, _: ()) -> Self::Future {
        let this = self.clone();
        async { Ok(this) }
    }
}

impl<F, Req, T, O, Res, Err> Service<Req> for HandlerService<F, T, O, Res, Err>
where
    F: for<'a> Handler<'a, Req, T, Response = O, Error = Err>,
    O: Responder<Req, Output = Res>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, mut req: Req) -> Self::Future<'_> {
        async move {
            let res = self.func.handle(&mut req).await?;
            Ok(res.respond_to(&mut req).await)
        }
    }
}

#[doc(hidden)]
/// Helper trait to make the HRTB bounds in `handler_service` as simple as posible.
///     `F: for<'a> Handler<'a, Req, T, Output=Out, Error=Err>,`
/// More precisely, HRTB bounds shouldn't specify any associated type that is bound to the
/// quantified lifetime.
/// For example, the following bounds would otherwise be necessary in `fn_service`:
///     `F: for<'a> FnArgs<<T as FromRequest>::Type<'a>>,`
///     `for<'a> FnArgs<T::Type<'a>>::Output: Future`
/// But these are known to be buggy. See https://github.com/rust-lang/rust/issues/56556
pub trait Handler<'a, Req, T>: Clone + 'static {
    type Error;
    type Response;
    type Future: Future<Output = Result<Self::Response, Self::Error>>
    where
        Req: 'a;

    fn handle(&'a self, req: &'a mut Req) -> Self::Future;
}

impl<'a, Req, T, Res, Err, F> Handler<'a, Req, T> for F
where
    T: FromRequest<'static, Req, Error = Err>,
    F: AsyncFn<T::Type<'a>, Output = Res> + Clone + 'static,
    F: AsyncFn<T>, // second bound to assist type inference to pinpoint T
{
    type Error = Err;
    type Response = Res;
    type Future = impl Future<Output = Result<Self::Response, Self::Error>> where Req: 'a;

    #[inline]
    fn handle(&'a self, req: &'a mut Req) -> Self::Future {
        async move {
            let extract = T::Type::<'a>::from_request(req).await?;
            Ok(self.call(extract).await)
        }
    }
}

/// Extract type from Req and receive them with function passed to [handler_service]
pub trait FromRequest<'a, Req>: Sized {
    // Used to construct the type for any lifetime 'b.
    //
    // Most extractors should set this to Self.
    // However extractor types with lifetime paramter that bwrrows from request and wants to to
    // support being extracted in handler arguments should take care.
    type Type<'b>: FromRequest<'b, Req, Error = Self::Error>;

    type Error;
    type Future: Future<Output = Result<Self, Self::Error>>
    where
        Req: 'a;

    fn from_request(req: &'a Req) -> Self::Future;
}

macro_rules! from_req_impl {
    ($fut: ident; $($req: ident),*) => {
        impl<'a, Req, Err, $($req,)*> FromRequest<'a, Req> for ($($req,)*)
        where
            $(
                $req: FromRequest<'a, Req, Error = Err>,
            )*
        {
            type Type<'r> = ($($req::Type<'r>,)*);
            type Error = Err;
            type Future = impl Future<Output = Result<Self, Self::Error>> where Req: 'a;

            fn from_request(req: &'a Req) -> Self::Future {
                $fut {
                    $(
                        $req: ExtractFuture::Future {
                            fut: $req::from_request(req)
                        },
                    )+
                }
            }
        }

        pin_project! {
            struct $fut<'f, Req: 'f, $($req: FromRequest<'f, Req>),+>
            {
                $(
                    #[pin]
                    $req: ExtractFuture<$req::Future, $req>,
                )+
            }
        }

        impl<'f, Req: 'f, Err, $($req: FromRequest<'f, Req, Error = Err>),+> Future for $fut<'f, Req, $($req),+>
        {
            type Output = Result<($($req,)+), Err>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let mut this = self.project();

                let mut ready = true;
                $(
                    match this.$req.as_mut().project() {
                        ExtractProj::Future { fut } => match fut.poll(cx)? {
                            Poll::Ready(output) => {
                                let _ = this.$req.as_mut().project_replace(ExtractFuture::Done { output });
                            },
                            Poll::Pending => ready = false,
                        },
                        ExtractProj::Done { .. } => {},
                        ExtractProj::Empty => unreachable!("FromRequest polled after finished"),
                    }
                )+

                if ready {
                    Poll::Ready(Ok(
                        ($(
                            match this.$req.project_replace(ExtractFuture::Empty) {
                                ExtractReplaceProj::Done { output } => output,
                                _ => unreachable!("FromRequest polled after finished"),
                            },
                        )+)
                    ))
                } else {
                    Poll::Pending
                }
            }
        }
    };
}

pin_project! {
    #[project = ExtractProj]
    #[project_replace = ExtractReplaceProj]
    enum ExtractFuture<Fut, Res> {
        Future {
            #[pin]
            fut: Fut
        },
        Done {
            output: Res,
        },
        Empty
    }
}

from_req_impl! { Extract1; A }
from_req_impl! { Extract2; A, B }
from_req_impl! { Extract3; A, B, C }
from_req_impl! { Extract4; A, B, C, D }
from_req_impl! { Extract5; A, B, C, D, E }
from_req_impl! { Extract6; A, B, C, D, E, F }
from_req_impl! { Extract7; A, B, C, D, E, F, G }
from_req_impl! { Extract8; A, B, C, D, E, F, G, H }
from_req_impl! { Extract9; A, B, C, D, E, F, G, H, I }

/// Make Response with reference of Req.
/// The Output type is what returns from [handler_service] function.
pub trait Responder<Req> {
    type Output;
    type Future<'a>: Future<Output = Self::Output>
    where
        Req: 'a;

    fn respond_to(self, req: &mut Req) -> Self::Future<'_>;
}

impl<R, T, E> Responder<R> for Result<T, E>
where
    T: Responder<R>,
{
    type Output = Result<T::Output, E>;
    type Future<'a> = impl Future<Output = Self::Output> where R: 'a;

    #[inline]
    fn respond_to(self, req: &mut R) -> Self::Future<'_> {
        async move { Ok(self?.respond_to(req).await) }
    }
}

#[doc(hidden)]
/// Same as `std::ops::Fn` trait but for async output.
///
/// It is necessary in the the HRTB bounds for async fn's with reference paramters because it
/// allows the output future to be bound to the paramter lifetime.
///     `F: for<'a> AsyncFn<(&'a u8,) Output=u8>`
pub trait AsyncFn<Arg> {
    type Output;
    type Future: Future<Output = Self::Output>;

    fn call(&self, arg: Arg) -> Self::Future;
}

macro_rules! async_fn_impl {
    ($($arg: ident),*) => {
        impl<Func, Fut, $($arg,)*> AsyncFn<($($arg,)*)> for Func
        where
            Func: Fn($($arg),*) -> Fut,
            Fut: Future,
        {
            type Output = Fut::Output;
            type Future = Fut;

            #[inline]
            fn call(&self, ($($arg,)*): ($($arg,)*)) -> Self::Future {
                self($($arg,)*)
            }
        }
    }
}

async_fn_impl! {}
async_fn_impl! { A }
async_fn_impl! { A, B }
async_fn_impl! { A, B, C }
async_fn_impl! { A, B, C, D }
async_fn_impl! { A, B, C, D, E }
async_fn_impl! { A, B, C, D, E, F }
async_fn_impl! { A, B, C, D, E, F, G }
async_fn_impl! { A, B, C, D, E, F, G, H }
async_fn_impl! { A, B, C, D, E, F, G, H, I }

#[cfg(test)]
mod test {
    use super::*;

    use std::{
        convert::Infallible,
        future::{ready, Ready},
    };

    use xitca_service::{BuildServiceExt, Service};

    use crate::http::{Response, StatusCode};
    use crate::util::service::{route::get, Router};
    use crate::Request;

    async fn handler(e1: String, e2: u32, (_, e3): (&Request<()>, u64)) -> StatusCode {
        assert_eq!(e1, "996");
        assert_eq!(e2, 996);
        assert_eq!(e3, 996);

        StatusCode::MULTI_STATUS
    }

    impl Responder<Request<()>> for StatusCode {
        type Output = Response<()>;
        type Future<'a> = impl Future<Output = Self::Output>;

        fn respond_to(self, _: &mut Request<()>) -> Self::Future<'_> {
            async move {
                let mut res = Response::new(());
                *res.status_mut() = self;
                res
            }
        }
    }

    impl<'a> FromRequest<'a, Request<()>> for String {
        type Type<'f> = Self;
        type Error = Infallible;
        type Future = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &'a Request<()>) -> Self::Future {
            ready(Ok(String::from("996")))
        }
    }

    impl<'a> FromRequest<'a, Request<()>> for u32 {
        type Type<'f> = Self;
        type Error = Infallible;
        type Future = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &'a Request<()>) -> Self::Future {
            ready(Ok(996))
        }
    }

    impl<'a> FromRequest<'a, Request<()>> for u64 {
        type Type<'f> = Self;
        type Error = Infallible;
        type Future = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &'a Request<()>) -> Self::Future {
            ready(Ok(996))
        }
    }

    impl<'a> FromRequest<'a, Request<()>> for &'a Request<()> {
        type Type<'f> = &'f Request<()>;
        type Error = Infallible;
        type Future = impl Future<Output = Result<Self, Self::Error>>;

        fn from_request(req: &'a Request<()>) -> Self::Future {
            async move { Ok(req) }
        }
    }

    #[tokio::test]
    async fn concurrent_extract_with_enclosed_fn() {
        async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
        where
            S: Service<Req>,
        {
            service.call(req).await
        }

        let res = handler_service(handler)
            .enclosed_fn(enclosed)
            .build(())
            .await
            .unwrap()
            .call(Request::new(()))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::MULTI_STATUS);
    }

    #[tokio::test]
    async fn handler_in_router() {
        let res = Router::new()
            .insert("/", get(handler_service(handler)))
            .build(())
            .await
            .unwrap()
            .call(Request::new(()))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::MULTI_STATUS);
    }
}
