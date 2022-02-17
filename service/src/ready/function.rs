use core::future::Future;

use crate::factory::function::FnServiceFactory;

use super::ReadyService;

impl<F, Req, Fut, Res, Err> ReadyService<Req> for FnServiceFactory<F>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Ready = ();
    type ReadyFuture<'f>
    where
        Self: 'f,
    = impl Future<Output = Result<Self::Ready, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async { Ok(()) }
    }
}
