#![allow(non_snake_case)]

use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use xitca_service::{fn_service, Service, ServiceFactory};

pub fn handler_service<F, Req, T, Res, Err>(
    func: F,
) -> impl ServiceFactory<
    Req,
    Response = Res,
    Error = Err,
    InitError = (),
    Config = (),
    Service = impl Service<Req, Response = Res, Error = Err> + Clone,
>
where
    F: for<'a> Handler<'a, Req, T, Response = Res, Error = Err>,
{
    fn_service(move |req| {
        let func = func.clone();
        let req = RefCell::new(req);

        // self-referential future
        async move { func.handle(&req).await }
    })
}

/// Helper trait to make the HRTB bounds in `handler_service` as simple as posible.
///     `F: for<'a> Handler<'a, Req, T, Output=Out, Error=Err>,`
/// More preciscely, HRTB bound shouldn't specify any associated type that is bound to the
/// quantified lifetime.
/// For example, the following bounds would otherwise be necessary in `fn_service`:
///     `F: for<'a> FnArgs<<T as FromRequest>::Type<'a>>,`
///     `for<'a> FnArgs<T::Type<'a>>::Output: Future`
/// But these are known to be buggy. See https://github.com/rust-lang/rust/issues/56556
pub trait Handler<'a, Req, T>: Clone {
    type Error;
    type Response;
    type Future: Future<Output = Result<Self::Response, Self::Error>>;

    fn handle(&'a self, req: &'a RefCell<Req>) -> Self::Future;
}

impl<'a, Req, T, Fut, Res, Err, F> Handler<'a, Req, T> for F
where
    F: FnArgs<T::Type<'a>, Output = Fut> + Clone,
    // second bound to assist type inference to pinpoint T
    F: FnArgs<T>,
    T: FromRequest<Req, Error = Err>,
    Fut: Future<Output = Res>,
{
    type Error = Err;
    type Response = Res;
    type Future = impl Future<Output = Result<Self::Response, Self::Error>> + 'a;

    fn handle(&'a self, req: &'a RefCell<Req>) -> Self::Future {
        async move { Ok(self.call(T::from_request(req).await?).await) }
    }
}

pub trait FromRequest<Req>: Sized {
    // Most extractors should set this to Self.
    //
    // However extractor types with lifetime paramter that bwrrows from request and wants to to
    // support being extracted in handler arguments should take care.
    type Type<'f>;

    type Error;
    type Future<'f>: Future<Output = Result<Self::Type<'f>, Self::Error>>
    where
        Req: 'f;

    fn from_request(req: &RefCell<Req>) -> Self::Future<'_>;
}

macro_rules! from_req_impl {
    ($fut: ident; $($req: ident),*) => {
        impl<Req, Err, $($req,)*> FromRequest<Req> for ($($req,)*)
        where
            $(
                $req: FromRequest<Req, Error = Err>,
            )*
        {
            type Type<'f> = ($($req::Type<'f>,)*);
            type Error = Err;
            type Future<'f>
            where
                Req: 'f,
            = impl Future<Output = Result<Self::Type<'f>, Self::Error>>;

            fn from_request<'a>(req: &'a RefCell<Req>) -> Self::Future<'a> {
                $fut::<'a, _, _, $($req,)*> {
                    $(
                        $req: ExtractFuture::Future {
                            fut: $req::from_request(req)
                        },
                    )+
                }
            }
        }

        pin_project! {
            struct $fut<'f, Req, Err, $($req: FromRequest<Req, Error = Err>),+>
            where
                Req: 'f
            {
                $(
                    #[pin]
                    $req: ExtractFuture<$req::Future<'f>, $req::Type<'f>>,
                )+
            }
        }

        impl<'f, Req, Err, $($req: FromRequest<Req, Error = Err>),+> Future for $fut<'f, Req, Err, $($req),+>
        {
            type Output = Result<($($req::Type<'f>,)+), Err>;

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

pub trait FnArgs<Arg>: Clone {
    type Output;

    fn call(&self, arg: Arg) -> Self::Output;
}

macro_rules! handler_impl {
    ($($arg: ident),*) => {
        impl<Func, O, $($arg,)*> FnArgs<($($arg,)*)> for Func
        where
            Func: Fn($($arg),*) -> O + Clone,
        {
            type Output = O;

            fn call(&self, ($($arg,)*): ($($arg,)*)) -> Self::Output {
                self($($arg,)*)
            }
        }
    }
}

handler_impl! {}
handler_impl! { A }
handler_impl! { A, B }
handler_impl! { A, B, C }
handler_impl! { A, B, C, D }
handler_impl! { A, B, C, D, E }
handler_impl! { A, B, C, D, E, F }
handler_impl! { A, B, C, D, E, F, G }
handler_impl! { A, B, C, D, E, F, G, H }
handler_impl! { A, B, C, D, E, F, G, H, I }

#[cfg(test)]
mod test {
    use super::*;

    use std::{
        convert::Infallible,
        future::{ready, Ready},
    };

    use xitca_service::{Service, ServiceFactoryExt};

    use crate::http::{Request, Response, StatusCode};

    async fn handler(e1: String, e2: u32, (_, e3): (&RefCell<Request<()>>, u64)) -> Response<()> {
        assert_eq!(e1, "996");
        assert_eq!(e2, 996);
        assert_eq!(e3, 996);

        let mut res = Response::new(());
        *res.status_mut() = StatusCode::MULTI_STATUS;
        res
    }

    impl FromRequest<Request<()>> for String {
        type Type<'f> = Self;
        type Error = Infallible;
        type Future<'f> = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &RefCell<Request<()>>) -> Self::Future<'_> {
            ready(Ok(String::from("996")))
        }
    }

    impl FromRequest<Request<()>> for u32 {
        type Type<'f> = Self;
        type Error = Infallible;
        type Future<'f> = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &RefCell<Request<()>>) -> Self::Future<'_> {
            ready(Ok(996))
        }
    }

    impl FromRequest<Request<()>> for u64 {
        type Type<'f> = Self;
        type Error = Infallible;
        type Future<'f> = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &RefCell<Request<()>>) -> Self::Future<'_> {
            ready(Ok(996))
        }
    }

    impl FromRequest<Request<()>> for &RefCell<Request<()>> {
        type Type<'f> = &'f RefCell<Request<()>>;
        type Error = Infallible;
        type Future<'f> = Ready<Result<Self::Type<'f>, Self::Error>>;

        fn from_request(req: &RefCell<Request<()>>) -> Self::Future<'_> {
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
