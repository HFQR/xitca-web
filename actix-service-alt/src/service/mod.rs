use core::{
    future::Future,
    task::{Context, Poll},
};

use alloc::{boxed::Box, rc::Rc};

pub trait Service {
    type Request<'r>;

    type Response;

    type Error;

    type Future<'f>: Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;

    fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s>;
}

impl<S: Service> Service for &'_ S {
    type Request<'r> = S::Request<'r>;
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = S::Future<'f>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        (*self).poll_ready(cx)
    }

    fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
        (*self).call(req)
    }
}

impl<S> Service for Box<S>
where
    S: Service + ?Sized,
{
    type Request<'r> = S::Request<'r>;
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = S::Future<'f>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        (**self).poll_ready(cx)
    }

    fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
        (**self).call(req)
    }
}

impl<S> Service for Rc<S>
where
    S: Service + ?Sized,
{
    type Request<'r> = S::Request<'r>;
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = S::Future<'f>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        (**self).poll_ready(cx)
    }

    fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
        (**self).call(req)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    struct NestService<S> {
        service: S,
    }

    impl<S> Service for NestService<S>
    where
        S: Service + 'static,
    {
        type Request<'r> = S::Request<'r>;
        type Response = S::Response;
        type Error = S::Error;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

        fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.service.poll_ready(cx)
        }

        fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
            async move { self.service.call(req).await }
        }
    }
}
