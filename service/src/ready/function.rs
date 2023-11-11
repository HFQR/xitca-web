use crate::service::FnService;

use super::ReadyService;

impl<F> ReadyService for FnService<F> {
    type Ready = ();

    #[inline]
    async fn ready(&self) -> Self::Ready {}
}
