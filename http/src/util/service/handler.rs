#![allow(non_snake_case)]

use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

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
    ($fut: ident; $(($n: tt, $req: ident)),*) => {
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
                    items: <($(Option<$req>,)+)>::default(),
                    $(
                        $req: $req::from_request(req),
                    )+
                }
            }
        }

        pin_project_lite::pin_project! {
                struct $fut<'f, Req, Err, $($req: FromRequest<Req, Error = Err>),+>
                where
                    Req: 'f
                {
                items: ($(Option<$req>,)+),
                $(
                    #[pin]
                    $req: $req::Future<'f>,
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
                    if this.items.$n.is_none() {
                        match this.$req.poll(cx)? {
                            Poll::Ready(item) => {
                                this.items.$n = Some(item);
                            }
                            Poll::Pending => ready = false,
                        }
                    }
                )+

                if ready {
                    Poll::Ready(Ok(
                        ($(this.items.$n.take().unwrap(),)+)
                    ))
                } else {
                    Poll::Pending
                }
            }
        }
    };
}

from_req_impl! { Extract1; (0, A) }
from_req_impl! { Extract2; (0, A), (1, B) }
from_req_impl! { Extract3; (0, A), (1, B), (2, C) }
from_req_impl! { Extract4; (0, A), (1, B), (2, C), (3, D) }
from_req_impl! { Extract5; (0, A), (1, B), (2, C), (3, D), (4, E) }
from_req_impl! { Extract6; (0, A), (1, B), (2, C), (3, D), (4, E), (5, F) }
from_req_impl! { Extract7; (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G) }
from_req_impl! { Extract8; (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H) }
from_req_impl! { Extract9; (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I) }

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
