use core::{
    future::Future,
    task::{Context, Poll},
};

use std::rc::Rc;

pub trait Service {
    type Request<'r>;

    type Response;

    type Error;

    type Future<'f>: Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;

    fn call<'s, 'r, 'f>(&'s self, req: Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f;
}

impl<S: Service> Service for &'_ S {
    type Request<'r> = S::Request<'r>;
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = S::Future<'f>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        (*self).poll_ready(cx)
    }

    fn call<'s, 'r, 'f>(&'s self, req: Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
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

    fn call<'s, 'r, 'f>(&'s self, req: Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
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

    fn call<'s, 'r, 'f>(&'s self, req: Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
        (**self).call(req)
    }
}
