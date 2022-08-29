use core::future::Future;

use crate::build::function::FnService;

use super::ReadyService;

impl<F> ReadyService for FnService<F>
where
    F: Clone,
{
    type Ready = ();
    type ReadyFuture<'f> = impl Future<Output = Self::Ready> where F: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async {}
    }
}
