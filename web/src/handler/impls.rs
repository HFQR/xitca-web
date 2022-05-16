use std::{convert::Infallible, error, future::Future, io};

use crate::{
    dev::bytes::Bytes,
    error::{MatchError, MethodNotAllowed},
    http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, StatusCode},
    request::WebRequest,
    response::WebResponse,
};

use super::{Extract, Inject};

impl<'a, 'r, S> Extract<'a, WebRequest<'r, S>> for &'a WebRequest<'a, S>
where
    S: 'static,
{
    type Type<'b> = &'b WebRequest<'b, S>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    #[inline]
    fn extract(req: &'a WebRequest<'r, S>) -> Self::Future {
        async move { Ok(&*req) }
    }
}

impl<'a, 'r, S> Extract<'a, WebRequest<'r, S>> for ()
where
    S: 'r,
{
    type Type<'b> = ();
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    #[inline]
    fn extract(_: &'a WebRequest<'r, S>) -> Self::Future {
        async { Ok(()) }
    }
}

impl<'r, S> Inject<WebRequest<'r, S>> for WebResponse {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    #[inline]
    fn inject(self, _: WebRequest<'r, S>) -> Self::Future {
        async { self }
    }
}

impl<'r, S: 'r> Inject<WebRequest<'r, S>> for () {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn inject(self, req: WebRequest<'r, S>) -> Self::Future {
        let res = req.into_response(Bytes::new());
        async { res }
    }
}

impl<'r, S: 'r> Inject<WebRequest<'r, S>> for Infallible {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn inject(self, _: WebRequest<'r, S>) -> Self::Future {
        async { unreachable!() }
    }
}

macro_rules! text_utf8 {
    ($type: ty) => {
        impl<'r, S: 'r> Inject<WebRequest<'r, S>> for $type {
            type Output = WebResponse;
            type Future = impl Future<Output = Self::Output>;

            fn inject(self, req: WebRequest<'r, S>) -> Self::Future {
                let mut res = req.into_response(self);
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
        impl<'r, S: 'r> Inject<WebRequest<'r, S>> for $type {
            type Output = WebResponse;
            type Future = impl Future<Output = Self::Output>;

            fn inject(self, req: WebRequest<'r, S>) -> Self::Future {
                let mut res = req.into_response(Bytes::new());
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

impl<'r, S> Inject<WebRequest<'r, S>> for MatchError
where
    S: 'r,
{
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn inject(self, req: WebRequest<'r, S>) -> Self::Future {
        let mut res = req.into_response(Bytes::new());
        *res.status_mut() = StatusCode::NOT_FOUND;
        async { res }
    }
}

impl<'r, S> Inject<WebRequest<'r, S>> for MethodNotAllowed
where
    S: 'r,
{
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn inject(self, req: WebRequest<'r, S>) -> Self::Future {
        let mut res = req.into_response(Bytes::new());
        *res.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        async { res }
    }
}
