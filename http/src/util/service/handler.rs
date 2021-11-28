#![allow(non_snake_case)]

use std::{convert::Infallible, future::Future};

use xitca_service::{fn_service, ServiceFactory};

use crate::http::Request;

pub fn handler_service<F, Req, T>(
    func: F,
) -> impl ServiceFactory<Req, Response = <F::Output as Future>::Output, Error = T::Error, InitError = (), Config = ()>
where
    F: Handler<T>,
    F::Output: Future,
    T: FromRequest<Req>,
{
    fn_service(move |mut req| {
        let func = func.clone();
        async move {
            let t = T::from_request(&mut req).await?;
            Ok(func.call(t).await)
        }
    })
}

pub trait FromRequest<Req>: Sized {
    type Error;
    type Future<'f>: Future<Output = Result<Self, Self::Error>>
    where
        Req: 'f;

    fn from_request(req: &mut Req) -> Self::Future<'_>;
}

macro_rules! from_req_impl {
    ($($req: ident),*) => {
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

            fn from_request(req: &mut Req) -> Self::Future<'_> {
                async move {
                    Ok((
                        $(
                            $req::from_request(req).await?,
                        )*
                    ))
                }
            }
        }
    };
}

from_req_impl! { A }
from_req_impl! { A, B }
from_req_impl! { A, B, C }
from_req_impl! { A, B, C, D }
from_req_impl! { A, B, C, D, E }
from_req_impl! { A, B, C, D, E, F }
from_req_impl! { A, B, C, D, E, F, G }
from_req_impl! { A, B, C, D, E, F, G, H }
from_req_impl! { A, B, C, D, E, F, G, H, I }

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
