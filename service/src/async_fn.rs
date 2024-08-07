#![allow(non_snake_case)]

use core::future::Future;

/// Same as `std::ops::Fn` trait but for async output.
///
/// It is necessary in the the HRTB bounds for async fn's with reference parameters because it
/// allows the output future to be bound to the parameter lifetime.
///     `F: for<'a> AsyncFn<(&'a u8,) Output=u8>`
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
