use std::{convert::Infallible, error, future::Future, io};

use crate::{
    dev::bytes::Bytes,
    error::{RouteError, RouterError},
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, StatusCode},
    request::WebRequest,
    response::WebResponse,
};

use super::{FromRequest, Responder};

impl<'a, 'r, S> FromRequest<'a, WebRequest<'r, S>> for &'a WebRequest<'a, S>
where
    S: 'static,
{
    type Type<'b> = &'b WebRequest<'b, S>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, S>) -> Self::Future {
        async move { Ok(&*req) }
    }
}

impl<'r, S> Responder<WebRequest<'r, S>> for WebResponse {
    type Output = WebResponse;
    type Future<'a> = impl Future<Output = Self::Output> where WebRequest<'r, S>: 'a;

    #[inline]
    fn respond_to<'a>(self, _: &'a mut WebRequest<'r, S>) -> Self::Future<'a> {
        async { self }
    }
}

impl<'r, S: 'r> Responder<WebRequest<'r, S>> for () {
    type Output = WebResponse;
    type Future<'a> = impl Future<Output = Self::Output> where WebRequest<'r, S>: 'a;

    fn respond_to<'a>(self, req: &'a mut WebRequest<'r, S>) -> Self::Future<'a> {
        let res = req.as_response(Bytes::new());
        async { res }
    }
}

impl<'r, S: 'r> Responder<WebRequest<'r, S>> for Infallible {
    type Output = WebResponse;
    type Future<'a> = impl Future<Output = Self::Output> where WebRequest<'r, S>: 'a;

    fn respond_to<'a>(self, _: &'a mut WebRequest<'r, S>) -> Self::Future<'a> {
        async { unreachable!() }
    }
}

macro_rules! text_utf8 {
    ($type: ty) => {
        impl<'r, S: 'r> Responder<WebRequest<'r, S>> for $type {
            type Output = WebResponse;
            type Future<'a> = impl Future<Output = Self::Output> where WebRequest<'r, S>: 'a;

            fn respond_to<'a>(self, req: &'a mut WebRequest<'r, S>) -> Self::Future<'a> {
                let mut res = req.as_response(self);
                res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
                async { res }
            }
        }
    };
}

text_utf8!(String);
text_utf8!(&'static str);

macro_rules! blank_internal {
    ($type: ty) => {
        impl<'r, S: 'r> Responder<WebRequest<'r, S>> for $type {
            type Output = WebResponse;
            type Future<'a> = impl Future<Output = Self::Output> where WebRequest<'r, S>: 'a;

            fn respond_to<'a>(self, req: &'a mut WebRequest<'r, S>) -> Self::Future<'a> {
                let mut res = req.as_response(Bytes::new());
                *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                async { res }
            }
        }
    };
}

blank_internal!(io::Error);
blank_internal!(Box<dyn error::Error>);
blank_internal!(Box<dyn error::Error + Send>);
blank_internal!(Box<dyn error::Error + Send + Sync>);

impl<'r, S, E> Responder<WebRequest<'r, S>> for RouterError<E>
where
    S: 'r,
    E: Responder<WebRequest<'r, S>, Output = WebResponse>,
{
    type Output = WebResponse;
    type Future<'a> = impl Future<Output = Self::Output> where WebRequest<'r, S>: 'a;

    fn respond_to<'a>(self, req: &'a mut WebRequest<'r, S>) -> Self::Future<'a> {
        async move {
            match self {
                Self::First(_) => {
                    let mut res = req.as_response(Bytes::new());
                    *res.status_mut() = StatusCode::NOT_FOUND;
                    res
                }
                Self::Second(e) => e.respond_to(req).await,
            }
        }
    }
}

impl<'r, S: 'r, E> Responder<WebRequest<'r, S>> for RouteError<E>
where
    S: 'r,
    E: Responder<WebRequest<'r, S>, Output = WebResponse>,
{
    type Output = WebResponse;
    type Future<'a> = impl Future<Output = Self::Output> where WebRequest<'r, S>: 'a;

    fn respond_to<'a>(self, req: &'a mut WebRequest<'r, S>) -> Self::Future<'a> {
        async move {
            match self {
                Self::First(_) => {
                    let mut res = req.as_response(Bytes::new());
                    *res.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
                    res
                }
                Self::Second(e) => e.respond_to(req).await,
            }
        }
    }
}
