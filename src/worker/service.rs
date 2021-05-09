use std::marker::PhantomData;
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_service::Service;

use super::limit::LimitGuard;

use crate::net::{FromStream, Stream};

pub(crate) type BoxedWorkerService = Box<dyn ServerService<(LimitGuard, Stream), Error = ()>>;

/// A special Service trait for actix_server.
/// The goal is to add clone_service method for trait object and simplify
/// associated type.
pub(crate) trait ServerService<Req> {
    type Error;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;

    fn call(&self, req: Req);

    fn clone_service(&self) -> Box<dyn ServerService<Req, Error = Self::Error>>;
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
    pub(crate) fn new_boxed(service: S) -> BoxedWorkerService {
        Box::new(WorkerService {
            service: Rc::new(service),
            _req: PhantomData,
        })
    }
}

impl<S, Req> Clone for WorkerService<S, Req>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            _req: PhantomData,
        }
    }
}

impl<S, Req> ServerService<(LimitGuard, Stream)> for WorkerService<S, Req>
where
    S: Service<Req> + Clone + 'static,
    Req: FromStream + 'static,
{
    type Error = ();

    #[inline]
    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
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

    fn clone_service(&self) -> Box<dyn ServerService<(LimitGuard, Stream), Error = Self::Error>> {
        Box::new(self.clone())
    }
}
