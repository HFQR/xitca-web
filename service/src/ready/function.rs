use core::future::Future;

use crate::build::function::FnService;

use super::ReadyService;

impl<F, Req, Fut, Res, Err> ReadyService<Req> for FnService<F>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Ready = ();
    type ReadyFuture<'f> = impl Future<Output = Self::Ready> where F: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async {}
    }
}
