use core::future::Future;

use crate::service::FnService;

use super::ReadyService;

impl<F> ReadyService for FnService<F> {
    type Ready = ();
    type Future<'f> = impl Future<Output = Self::Ready> where Self: 'f;

    #[inline]
    fn ready(&self) -> Self::Future<'_> {
        async {}
    }
}
