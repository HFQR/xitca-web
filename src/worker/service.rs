use std::{
    future::{ready, Ready},
    marker::PhantomData,
    rc::Rc,
    task::{Context, Poll},
};

use actix_service::Service;

use super::limit::LimitGuard;

use crate::net::{FromStream, Stream};

pub(crate) type RcWorkerService = Rc<
    dyn Service<(LimitGuard, Stream), Response = (), Error = (), Future = Ready<Result<(), ()>>>,
>;

pub(crate) struct WorkerService<S, Req> {
    service: S,
    _req: PhantomData<Req>,
}

impl<S, Req> WorkerService<S, Req>
where
    S: Service<Req> + 'static,
    Req: FromStream + 'static,
{
    pub(crate) fn new_rcboxed(service: S) -> RcWorkerService {
        Rc::new(WorkerService {
            service,
            _req: PhantomData,
        })
    }
}

impl<S, Req> Service<(LimitGuard, Stream)> for WorkerService<S, Req>
where
    S: Service<Req> + 'static,
    Req: FromStream + 'static,
{
    type Response = ();

    type Error = ();

    type Future = Ready<Result<(), ()>>;

    #[inline]
    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx).map_err(|_| ())
    }

    fn call(&self, (guard, req): (LimitGuard, Stream)) -> Self::Future {
        let stream = FromStream::from_stream(req);
        let service = self.service.call(stream);

        tokio::task::spawn_local(async move {
            let _ = service.await;
            drop(guard);
        });

        ready(Ok(()))
    }
}
