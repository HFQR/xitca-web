use core::{convert::Infallible, future::Future, marker::PhantomData};

use super::Service;

pub fn fn_service<F, Req, Fut, Res, Err>(f: F) -> FnService<F, BuildFnService>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    FnService {
        f,
        _marker: PhantomData,
    }
}

pub fn fn_build<F, Arg, Fut, Svc, Err>(f: F) -> FnBuild<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
{
    FnBuild { f }
}

#[derive(Clone)]
pub struct FnBuild<F> {
    f: F,
}

impl<F, Arg, Fut, Svc, Err> Service<Arg> for FnBuild<F>
where
    F: Fn(Arg) -> Fut,
    Fut: Future<Output = Result<Svc, Err>>,
{
    type Response = Svc;
    type Error = Err;
    type Future<'f> = Fut where Self: 'f;

    fn call(&self, arg: Arg) -> Self::Future<'_> {
        (self.f)(arg)
    }
}

pub struct FnService<F, M> {
    f: F,
    _marker: PhantomData<M>,
}

impl<F, M> Clone for FnService<F, M>
where
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            f: self.f.clone(),
            _marker: PhantomData,
        }
    }
}

pub struct BuildFnService;
pub struct CallFnServie;

impl<F> Service for FnService<F, BuildFnService>
where
    F: Clone,
{
    type Response = FnService<F, CallFnServie>;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    fn call(&self, _: ()) -> Self::Future<'_> {
        let f = self.f.clone();
        async {
            Ok(FnService {
                f,
                _marker: PhantomData,
            })
        }
    }
}

impl<F, Req, Fut, Res, Err> Service<Req> for FnService<F, CallFnServie>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = Fut where Self: 'f;

    #[inline]
    fn call(&self, req: Req) -> Self::Future<'_> {
        (self.f)(req)
    }
}
