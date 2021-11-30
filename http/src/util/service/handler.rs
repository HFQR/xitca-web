#![allow(non_snake_case)]

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use xitca_service::{Service, ServiceFactory};

pub fn handler_service<'t, F, Req, T, O, Res, Err>(func: F) -> HandlerService<'t, F, T, O, Res, Err>
where
    F: for<'a> Handler<'t, 'a, Req, T, Response = O, Error = Err>,
    O: for<'a> Responder<'a, Req, Output = Res>,
{
    HandlerService::new(func)
}

pub struct HandlerService<'t, F, T, O, Res, Err> {
    func: F,
    _p: PhantomData<(&'t (), T, O, Res, Err)>,
}

impl<'t, F, T, O, Res, Err> HandlerService<'t, F, T, O, Res, Err> {
    pub fn new<Req>(func: F) -> Self
    where
        F: for<'a> Handler<'t, 'a, Req, T, Response = O, Error = Err>,
        O: for<'a> Responder<'a, Req, Output = Res>,
    {
        Self { func, _p: PhantomData }
    }
}

impl<F, T, O, Res, Err> Clone for HandlerService<'_, F, T, O, Res, Err>
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

impl<'t, F, Req, T, O, Res, Err> ServiceFactory<Req> for HandlerService<'t, F, T, O, Res, Err>
where
    F: for<'a> Handler<'t, 'a, Req, T, Response = O, Error = Err>,
    O: for<'a> Responder<'a, Req, Output = Res>,
{
    type Response = Res;
    type Error = Err;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let this = self.clone();
        async { Ok(this) }
    }
}

impl<'t, F, Req, T, O, Res, Err> Service<Req> for HandlerService<'t, F, T, O, Res, Err>
where
    F: for<'a> Handler<'t, 'a, Req, T, Response = O, Error = Err>,
    O: for<'a> Responder<'a, Req, Output = Res>,
{
    type Response = Res;
    type Error = Err;
    type Ready<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<(), Self::Error>>;
    type Future<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        async { Ok(()) }
    }

    #[inline]
    fn call(&self, mut req: Req) -> Self::Future<'_> {
        async move {
            let res = self.func.handle(&mut req).await?;
            Ok(res.respond_to(&mut req).await)
        }
    }
}

/// Helper trait to make the HRTB bounds in `handler_service` as simple as posible.
///     `F: for<'a> Handler<'a, Req, T, Output=Out, Error=Err>,`
/// More precisely, HRTB bounds shouldn't specify any associated type that is bound to the
/// quantified lifetime.
/// For example, the following bounds would otherwise be necessary in `fn_service`:
///     `F: for<'a> FnArgs<<T as FromRequest>::Type<'a>>,`
///     `for<'a> FnArgs<T::Type<'a>>::Output: Future`
/// But these are known to be buggy. See https://github.com/rust-lang/rust/issues/56556
pub trait Handler<'t, 'a, Req, T>: Clone {
    type Error;
    type Response;
    type Future: Future<Output = Result<Self::Response, Self::Error>>;

    fn handle(&'a self, req: &'a mut Req) -> Self::Future;
}

impl<'t, 'a, Req, T, Res, Err, F> Handler<'t, 'a, Req, T> for F
where
    T: FromRequest<'t, Req, Error = Err>,
    F: AsyncFn<T::Type<'a>, Output = Res> + Clone,
    F: AsyncFn<T>, // second bound to assist type inference to pinpoint T
{
    type Error = Err;
    type Response = Res;
    type Future = impl Future<Output = Result<Self::Response, Self::Error>> + 'a;

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
    type Future: Future<Output = Result<Self, Self::Error>>;

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
            type Future = impl Future<Output = Result<Self, Self::Error>>;

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
            struct $fut<'f, Req, Err, $($req: FromRequest<'f, Req, Error = Err>),+>
            {
                $(
                    #[pin]
                    $req: ExtractFuture<$req::Future, $req>,
                )+
            }
        }

        impl<'f, Req, Err, $($req: FromRequest<'f, Req, Error = Err>),+> Future for $fut<'f, Req, Err, $($req),+>
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
pub trait Responder<'a, Req> {
    type Output;
    type Future: Future<Output = Self::Output>;

    fn respond_to(self, req: &'a mut Req) -> Self::Future;
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

    use xitca_service::{Service, ServiceFactoryExt};

    use crate::http::{Request, Response, StatusCode};

    async fn handler(e1: String, e2: u32, (_, e3): (&Request<()>, u64)) -> StatusCode {
        assert_eq!(e1, "996");
        assert_eq!(e2, 996);
        assert_eq!(e3, 996);

        StatusCode::MULTI_STATUS
    }

    impl<'a> Responder<'a, Request<()>> for StatusCode {
        type Output = Response<()>;
        type Future = Ready<Self::Output>;

        fn respond_to(self, _: &'a mut Request<()>) -> Self::Future {
            let mut res = Response::new(());
            *res.status_mut() = self;
            ready(res)
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
        type Future = Ready<Result<Self, Self::Error>>;

        fn from_request(req: &'a Request<()>) -> Self::Future {
            ready(Ok(req))
        }
    }

    #[tokio::test]
    async fn concurrent_extract() {
        let service = handler_service(handler)
            .transform_fn(|s, req| async move { s.call(req).await })
            .new_service(())
            .await
            .unwrap();

        let req = Request::new(());

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::MULTI_STATUS);
    }
}
