use std::{
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use actix_http_alt::HttpParts;
use actix_http_alt::{HttpResponse, RequestBody};
use actix_service_alt::Service;

use crate::extract::FromRequest;
use crate::request::WebRequest;
use crate::response::WebResponse;

/// A request handler is an async function that accepts zero or more parameters that can be
/// extracted from a request (i.e., [`impl FromRequest`](crate::FromRequest)) and returns a type
/// that can be converted into an [`HttpResponse`] (that is, it impls the [`Responder`] trait).
///
/// If you got the error `the trait Handler<_, _, _> is not implemented`, then your function is not
/// a valid handler. See [Request Handlers](https://actix.rs/docs/handlers/) for more information.
pub trait Handler<T, R>: Clone + 'static
where
    R: Future,
{
    fn call(&self, param: T) -> R;
}

#[doc(hidden)]
/// Extract arguments from request, run factory function and make response.
pub struct HandlerService<D, F, T, R>
where
    D: 'static,
    F: Handler<T, R>,
    T: for<'f> FromRequest<'f, D>,
    R: Future,
    R::Output: Responder<D>,
{
    hnd: F,
    _phantom: PhantomData<(D, T, R)>,
}

impl<D, F, T, R> Service for HandlerService<D, F, T, R>
where
    D: 'static,
    F: Handler<T, R>,
    T: for<'f> FromRequest<'f, D> + 'static,
    R: Future + 'static,
    R::Output: Responder<D>,
{
    type Request<'r> = WebRequest<'r, D>;
    type Response = WebResponse;
    type Error = ();
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
        async move {
            let extract = T::from_request(&req).await.map_err(|_| ())?;
            self.hnd.call(extract).await.respond_to(&req)
        }
    }
}

pub trait Responder<D>: Sized {
    fn respond_to(self, req: &WebRequest<'_, D>) -> Result<WebResponse, ()> {
        Err(())
    }
}
