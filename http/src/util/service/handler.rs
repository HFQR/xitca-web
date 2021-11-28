#![allow(non_snake_case)]

use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use xitca_service::{fn_service, ServiceFactory};

pub fn handler_service<F, Req, T>(
    func: F,
) -> impl ServiceFactory<Req, Response = <F::Output as Future>::Output, Error = T::Error, InitError = (), Config = ()>
where
    F: Handler<T>,
    F::Output: Future,
    T: FromRequest<Req>,
{
    fn_service(move |req| {
        let func = func.clone();
        let req = RefCell::new(req);
        async move {
            let t = T::from_request(&req).await?;
            Ok(func.call(t).await)
        }
    })
}

pub trait FromRequest<Req>: Sized {
    type Error;
    type Future<'f>: Future<Output = Result<Self, Self::Error>>
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
            type Error = Err;
            type Future<'f>
            where
                Req: 'f,
            = impl Future<Output = Result<Self, Self::Error>>;

            fn from_request(req: &RefCell<Req>) -> Self::Future<'_> {
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
            struct $fut<'f, Req, Err, $($req: FromRequest<Req, Error = Err>),+>
            where
                Req: 'f
            {
                $(
                    #[pin]
                    $req: ExtractFuture<$req::Future<'f>, $req>,
                )+
            }
        }

        impl<Req, Err, $($req: FromRequest<Req, Error = Err>),+> Future for $fut<'_, Req, Err, $($req),+>
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

pub trait Handler<Arg>: Clone {
    type Output;

    fn call(&self, arg: Arg) -> Self::Output;
}

macro_rules! handler_impl {
    ($($arg: ident),*) => {
        impl<Func, O, $($arg,)*> Handler<($($arg,)*)> for Func
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

    use xitca_service::Service;

    use crate::http::{Request, Response, StatusCode};

    async fn handler(e1: String, e2: u32, e3: u64) -> Response<()> {
        assert_eq!(e1, "996");
        assert_eq!(e2, 996);
        assert_eq!(e3, 996);

        let mut res = Response::new(());
        *res.status_mut() = StatusCode::MULTI_STATUS;
        res
    }

    impl FromRequest<Request<()>> for String {
        type Error = Infallible;
        type Future<'f> = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &RefCell<Request<()>>) -> Self::Future<'_> {
            ready(Ok(String::from("996")))
        }
    }

    impl FromRequest<Request<()>> for u32 {
        type Error = Infallible;
        type Future<'f> = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &RefCell<Request<()>>) -> Self::Future<'_> {
            ready(Ok(996))
        }
    }

    impl FromRequest<Request<()>> for u64 {
        type Error = Infallible;
        type Future<'f> = Ready<Result<Self, Self::Error>>;

        fn from_request(_: &RefCell<Request<()>>) -> Self::Future<'_> {
            ready(Ok(996))
        }
    }

    #[tokio::test]
    async fn concurrent_extract() {
        let service = handler_service(handler).new_service(()).await.unwrap();

        let req = Request::new(());

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::MULTI_STATUS);
    }
}
