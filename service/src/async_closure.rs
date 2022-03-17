use core::future::Future;

pub trait AsyncClosure<Args> {
    type Output;
    type Future: Future<Output = Self::Output>;

    fn call(&self, arg: Args) -> Self::Future;
}

impl<F, Arg1, Arg2, Fut> AsyncClosure<(Arg1, Arg2)> for F
where
    F: Fn(Arg1, Arg2) -> Fut,
    Fut: Future,
{
    type Output = Fut::Output;
    type Future = impl Future<Output = Self::Output>;

    fn call(&self, (arg1, arg2): (Arg1, Arg2)) -> Self::Future {
        (self)(arg1, arg2)
    }
}
