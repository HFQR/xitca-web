//! middleware for catching panic.

use core::any::Any;

use xitca_service::{Service, pipeline::PipelineE};

/// builder for middleware catching panic and unwind it to [`CatchUnwindError`].
pub struct CatchUnwind;

impl<S, E> Service<Result<S, E>> for CatchUnwind {
    type Response = service::CatchUnwindService<S>;
    type Error = E;

    async fn call(&self, arg: Result<S, E>) -> Result<Self::Response, Self::Error> {
        arg.map(service::CatchUnwindService)
    }
}

/// type alias for branched catch unwind error. The First variant is panic message and the
/// Second variant is Service::Error produced by inner/next service CatchUnwind enclosed.
pub type CatchUnwindError<E> = PipelineE<Box<dyn Any + Send>, E>;

mod service {
    use core::panic::AssertUnwindSafe;

    use xitca_service::ready::ReadyService;
    use xitca_unsafe_collection::futures::CatchUnwind;

    use super::*;

    pub struct CatchUnwindService<S>(pub(super) S);

    impl<S, Req> Service<Req> for CatchUnwindService<S>
    where
        S: Service<Req>,
    {
        type Response = S::Response;
        type Error = CatchUnwindError<S::Error>;

        async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
            CatchUnwind::new(AssertUnwindSafe(self.0.call(req)))
                .await
                .map_err(CatchUnwindError::First)?
                .map_err(CatchUnwindError::Second)
        }
    }

    impl<S> ReadyService for CatchUnwindService<S>
    where
        S: ReadyService,
    {
        type Ready = S::Ready;

        #[inline]
        async fn ready(&self) -> Self::Ready {
            self.0.ready().await
        }
    }
}
