#![allow(non_snake_case)]

use core::future::Future;

// TODO: use `std::ops::AsyncFn` instead
/// Similar to `std::ops::AsyncFn` except the associated types are named and their trait bounds can be annotated.
/// It's necessary to express `Send` bound on the `Future` associated type.
pub trait AsyncFn<Arg> {
    type Output;
    type Future: Future<Output = Self::Output>;

    fn call(&self, arg: Arg) -> Self::Future;
}

macro_rules! async_closure_impl {
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

async_closure_impl! {}
async_closure_impl! { A }
async_closure_impl! { A, B }
async_closure_impl! { A, B, C }
async_closure_impl! { A, B, C, D }
async_closure_impl! { A, B, C, D, E }
async_closure_impl! { A, B, C, D, E, F }
async_closure_impl! { A, B, C, D, E, F, G }
async_closure_impl! { A, B, C, D, E, F, G, H }
async_closure_impl! { A, B, C, D, E, F, G, H, I }
