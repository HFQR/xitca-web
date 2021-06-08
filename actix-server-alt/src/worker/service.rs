use std::{
    marker::PhantomData,
    rc::Rc,
    task::{Context, Poll},
};

use actix_service_alt::Service;

use super::limit::LimitGuard;

use crate::net::{FromStream, Stream};

pub(crate) trait WorkerServiceTrait {
    fn poll_ready(&self, cx: &mut Context) -> Poll<Result<(), ()>>;

    fn call(self: Rc<Self>, req: (LimitGuard, Stream));
}

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

pub(crate) type RcWorkerService = Rc<dyn WorkerServiceTrait>;

impl<S, Req> WorkerServiceTrait for WorkerService<S, Req>
where
    S: Service<Req> + 'static,
    Req: FromStream + 'static,
{
    #[inline]
    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), ()>> {
        self.service.poll_ready(ctx).map_err(|_| ())
    }

    fn call(self: Rc<Self>, (guard, req): (LimitGuard, Stream)) {
        let stream = FromStream::from_stream(req);

        tokio::task::spawn_local(async move {
            let _ = self.service.call(stream).await;
            drop(guard);
        });
    }
}
