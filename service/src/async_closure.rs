use core::future::Future;

pub trait AsyncClosure<'s, S, Req> {
    type Output;
    type Future: Future<Output = Self::Output> + 's;

    fn call(&self, service: &'s S, req: Req) -> Self::Future;
}

impl<'s, F, Fut, S, Req> AsyncClosure<'s, S, Req> for F
where
    F: Fn(&'s S, Req) -> Fut + 's,
    S: 's,
    Fut: Future + 's,
    Req: 's,
{
    type Output = Fut::Output;
    type Future = impl Future<Output = Self::Output> + 's;

    fn call(&self, service: &'s S, req: Req) -> Self::Future {
        (self)(service, req)
    }
}
