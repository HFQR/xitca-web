#![allow(non_snake_case)]

use std::{
    convert::Infallible,
    future::{ready, Future},
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use xitca_service::{fn_build, pipeline::PipelineE, AsyncClosure, Service};

/// A service factory shortcut offering given async function ability to use [FromRequest] to destruct and transform `Service<Req>`'s
/// `Req` type and receive them as function argument.
///
/// Given async function's return type must impl [Responder] trait for transforming arbitrary return type to `Service::Future`'s
/// output type.
pub fn handler_service<F, T, O>(func: F) -> impl Service<Response = HandlerService<F, T, O>, Error = Infallible>
where
    F: Clone,
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
    T: FromRequest<'static, Req>,
    F: AsyncClosure<T>, // just to assist type inference to pinpoint `T`

    F: for<'a> AsyncClosure<T::Type<'a>, Output = O> + Clone + 'static,
    O: Responder<Req>,
{
    type Response = O::Output;
    type Error = T::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let extract = T::Type::<'_>::from_request(&req).await?;
            let res = self.func.call(extract).await;
            Ok(res.respond_to(req).await)
        }
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
/// #![feature(generic_associated_types, type_alias_impl_trait)]
/// # use std::future::Future;
/// # use xitca_http::util::service::handler::FromRequest;
/// struct MyExtractor<'a>(&'a str);
///
/// impl<'a, 'r> FromRequest<'a, &'r String> for MyExtractor<'a> {
///     type Type<'b> = MyExtractor<'b>;
///     type Error = ();
///     type Future = impl Future<Output = Result<Self, Self::Error>> where 'r: 'a;
///     fn from_request(req: &'a &'r String) -> Self::Future {
///         async { Ok(MyExtractor(req)) }
///     }
/// }
/// ```
pub trait FromRequest<'a, Req>: Sized {
    // Used to construct the type for any lifetime 'b.
    type Type<'b>: FromRequest<'b, Req, Error = Self::Error>;

    type Error;
    type Future: Future<Output = Result<Self, Self::Error>>
    where
        Req: 'a;

    fn from_request(req: &'a Req) -> Self::Future;
}

macro_rules! from_req_impl {
    ($fut: ident; $req0: ident, $($req: ident,)*) => {
        from_req_impl! { $fut, $req0, $($req,)* }

        impl<'a, Req, $req0, $($req,)*> FromRequest<'a, Req> for ($req0, $($req,)*)
        where
            $req0: FromRequest<'a, Req>,
            $(
                $req: FromRequest<'a, Req>,
                $req0::Error: From<$req::Error>,
            )*
        {
            type Type<'r> = ($req0::Type<'r>, $($req::Type<'r>,)*);
            type Error = $req0::Error;
            type Future = impl Future<Output = Result<Self, Self::Error>> where Req: 'a;

            fn from_request(req: &'a Req) -> Self::Future {
                $fut::new(req)
            }
        }

        impl<'f, Req, $req0, $($req,)*> Future for $fut<'f, Req, $req0, $($req,)*>
        where
            Req: 'f,
            $req0: FromRequest<'f, Req>,
            $(
                $req: FromRequest<'f, Req>,
                $req0::Error: From<$req::Error>,
            )*
        {
            type Output = Result<($req0, $($req,)*), $req0::Error>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let mut this = self.project();

                let mut ready = true;

                match this.$req0.as_mut().project() {
                    ExtractProj::Future { fut } => match fut.poll(cx)? {
                        Poll::Ready(output) => {
                            let _ = this.$req0.as_mut().project_replace(ExtractFuture::Done { output });
                        },
                        Poll::Pending => ready = false,
                    },
                    ExtractProj::Done { .. } => {},
                    ExtractProj::Empty => unreachable!("FromRequest polled after finished"),
                }

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
                )*

                if ready {
                    Poll::Ready(Ok(
                        (
                            match this.$req0.project_replace(ExtractFuture::Empty) {
                                ExtractReplaceProj::Done { output } => output,
                                _ => unreachable!("FromRequest polled after finished"),
                            },
                            $(
                                match this.$req.project_replace(ExtractFuture::Empty) {
                                    ExtractReplaceProj::Done { output } => output,
                                    _ => unreachable!("FromRequest polled after finished"),
                                },
                            )*
                        )
                    ))
                } else {
                    Poll::Pending
                }
            }
        }
    };
    ($fut: ident, $($req: ident,)*) => {
        pin_project! {
            struct $fut<'f, Req, $($req,)*>
            where
                Req: 'f,
                $(
                    $req: FromRequest<'f, Req>,
                )*
            {
                $(
                    #[pin]
                    $req: ExtractFuture<$req::Future, $req>,
                )*
            }
        }

        impl<'f, Req, $($req,)*> $fut<'f, Req, $($req,)*>
        where
            Req: 'f,
            $(
                $req: FromRequest<'f, Req>,
            )*
        {
            fn new(req: &'f Req) -> Self {
                Self {
                    $(
                        $req: ExtractFuture::Future {
                            fut: $req::from_request(req)
                        },
                    )*
                }
            }
        }
    }
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

from_req_impl! { Extract1; A, }
from_req_impl! { Extract2; A, B, }
from_req_impl! { Extract3; A, B, C, }
from_req_impl! { Extract4; A, B, C, D, }
from_req_impl! { Extract5; A, B, C, D, E, }
from_req_impl! { Extract6; A, B, C, D, E, F, }
from_req_impl! { Extract7; A, B, C, D, E, F, G, }
from_req_impl! { Extract8; A, B, C, D, E, F, G, H, }
from_req_impl! { Extract9; A, B, C, D, E, F, G, H, I, }

/// Make Response with reference of Req.
/// The Output type is what returns from [handler_service] function.
pub trait Responder<Req> {
    type Output;
    type Future: Future<Output = Self::Output>;

    fn respond_to(self, req: Req) -> Self::Future;
}

impl<R, T, E> Responder<R> for Result<T, E>
where
    T: Responder<R>,
{
    type Output = Result<T::Output, E>;
    type Future = impl Future<Output = Self::Output>;

    #[inline]
    fn respond_to(self, req: R) -> Self::Future {
        async { Ok(self?.respond_to(req).await) }
    }
}

impl<R, F, S> Responder<R> for PipelineE<F, S>
where
    F: Responder<R>,
    S: Responder<R, Output = F::Output>,
{
    type Output = F::Output;
    type Future = impl Future<Output = Self::Output>;

    #[inline]
    fn respond_to(self, req: R) -> Self::Future {
        async {
            match self {
                Self::First(f) => f.respond_to(req).await,
                Self::Second(s) => s.respond_to(req).await,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        convert::Infallible,
        future::{ready, Ready},
    };

    use xitca_service::{Service, ServiceExt};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        http::StatusCode,
        request::Request,
        response::Response,
        util::service::{route::get, Router},
    };

    use super::*;

    async fn handler(e1: String, e2: u32, (_, e3): (&Request<()>, u64)) -> StatusCode {
        assert_eq!(e1, "996");
        assert_eq!(e2, 996);
        assert_eq!(e3, 996);

        StatusCode::MULTI_STATUS
    }

    impl Responder<Request<()>> for StatusCode {
        type Output = Response<()>;
        type Future = impl Future<Output = Self::Output>;

        fn respond_to(self, _: Request<()>) -> Self::Future {
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
            .call(Request::new(()))
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status(), StatusCode::MULTI_STATUS);
    }

    #[test]
    fn handler_in_router() {
        let res = Router::new()
            .insert("/", get(handler_service(handler)))
            .call(())
            .now_or_panic()
            .unwrap()
            .call(Request::new(()))
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status(), StatusCode::MULTI_STATUS);
    }
}
