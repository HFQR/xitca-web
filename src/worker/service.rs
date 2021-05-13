use std::{
    rc::Rc,
    task::{Context, Poll},
};

use crate::service::Service;

use super::limit::LimitGuard;

use crate::net::{FromStream, Stream};

pub(crate) trait WorkerServiceTrait {
    fn poll_ready(&self, cx: &mut Context) -> Poll<Result<(), ()>>;

    fn call(&self, req: (LimitGuard, Stream));
}

pub(crate) struct WorkerService<S> {
    service: S,
}

impl<S> WorkerService<S>
where
    S: Service + 'static,
    S::Request<'static>: FromStream + 'static,
{
    pub(crate) fn new_rcboxed(service: S) -> RcWorkerService {
        Rc::new(WorkerService {
            service: Rc::new(service),
        })
    }
}

pub(crate) type RcWorkerService = Rc<dyn WorkerServiceTrait>;

impl<S> WorkerServiceTrait for WorkerService<S>
where
    S: Service + Clone + 'static,
    S::Request<'static>: FromStream + 'static,
{
    #[inline]
    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), ()>> {
        self.service.poll_ready(ctx).map_err(|_| ())
    }

    fn call(&self, (guard, req): (LimitGuard, Stream)) {
        let stream = FromStream::from_stream(req);
        let service = self.service.clone();

        tokio::task::spawn_local(async move {
            let _ = service.call(stream).await;
            drop(guard);
        });
    }
}
