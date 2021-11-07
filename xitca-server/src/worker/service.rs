use std::{marker::PhantomData, rc::Rc};

use xitca_service::Service;

use super::limit::LimitGuard;

use crate::net::{FromStream, Stream};

pub(crate) trait WorkerServiceTrait {
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
    fn call(self: Rc<Self>, (guard, req): (LimitGuard, Stream)) {
        let stream = FromStream::from_stream(req);

        tokio::task::spawn_local(async move {
            if self.service.ready().await.is_ok() {
                let _ = self.service.call(stream).await;
                drop(guard);
            }
        });
    }
}
