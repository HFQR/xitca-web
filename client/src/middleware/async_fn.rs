use crate::{
    error::Error,
    response::Response,
    service::{async_fn, Service, ServiceRequest},
};

/// middleware to wrap async function as a service.
pub struct AsyncFn<S, F> {
    service: S,
    func: F,
}

impl<S, F> AsyncFn<S, F> {
    pub fn new(service: S, func: F) -> Self {
        Self { service, func }
    }
}

impl<'r, 'c, S, F> Service<ServiceRequest<'r, 'c>> for AsyncFn<S, F>
where
    S: for<'r2, 'c2> Service<ServiceRequest<'r, 'c>, Response = Response, Error = Error> + Send + Sync,
    F: for<'r3, 'c3, 's3> async_fn::AsyncFn<(ServiceRequest<'r, 'c>, &'s3 S), Output = Result<Response, Error>>
        + Send
        + Sync,
    for<'r4, 'c4, 's4> <F as async_fn::AsyncFn<(ServiceRequest<'r4, 'c4>, &'s4 S)>>::Future: Send,
{
    type Response = Response;
    type Error = Error;

    async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
        self.func.call((req, &self.service)).await
    }
}
