use core::{convert::Infallible, future::Future};

use std::{error, io};

use crate::{
    body::BodyStream,
    dev::bytes::Bytes,
    error::{MatchError, MethodNotAllowed},
    http::{
        const_header_value::TEXT_UTF8,
        header::{ALLOW, CONTENT_TYPE},
        StatusCode,
    },
    request::WebRequest,
    response::WebResponse,
};

use super::{error::ExtractError, FromRequest, Responder};

impl<'a, 'r, C, B, T, E> FromRequest<'a, WebRequest<'r, C, B>> for Result<T, E>
where
    B: BodyStream,
    T: for<'a2, 'r2> FromRequest<'a2, WebRequest<'r2, C, B>, Error = E>,
{
    type Type<'b> = Result<T, E>;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r,  C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async { Ok(T::from_request(req).await) }
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebRequest<'r, C, B>> for Option<T>
where
    B: BodyStream,
    T: for<'a2, 'r2> FromRequest<'a2, WebRequest<'r2, C, B>>,
{
    type Type<'b> = Option<T>;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B, >: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async { Ok(T::from_request(req).await.ok()) }
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for &'a WebRequest<'a, C, B>
where
    C: 'static,
    B: BodyStream + 'static,
{
    type Type<'b> = &'b WebRequest<'b, C, B>;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async move { Ok(req) }
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebRequest<'r, C, B>> for ()
where
    B: BodyStream,
{
    type Type<'b> = ();
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(_: &'a WebRequest<'r, C, B>) -> Self::Future {
        async { Ok(()) }
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for WebResponse {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    #[inline]
    fn respond_to(self, _: WebRequest<'r, C, B>) -> Self::Future {
        async { self }
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for () {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
        let res = req.into_response(Bytes::new());
        async { res }
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for Infallible {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, _: WebRequest<'r, C, B>) -> Self::Future {
        async { match self {} }
    }
}

macro_rules! text_utf8 {
    ($type: ty) => {
        impl<'r, C, B> Responder<WebRequest<'r, C, B>> for $type {
            type Output = WebResponse;
            type Future = impl Future<Output = Self::Output>;

            fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
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
        impl<'r, C, B> Responder<WebRequest<'r, C, B>> for $type {
            type Output = WebResponse;
            type Future = impl Future<Output = Self::Output>;

            fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
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

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for MatchError {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
        let mut res = req.into_response(Bytes::new());
        *res.status_mut() = StatusCode::NOT_FOUND;
        async { res }
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for MethodNotAllowed {
    type Output = WebResponse;
    type Future = impl Future<Output = Self::Output>;

    fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Future {
        let mut res = req.into_response(Bytes::new());

        let allowed = self.allowed_methods();

        let len = allowed.iter().fold(0, |a, m| a + m.as_str().len() + 1);

        let mut methods = String::with_capacity(len);

        for method in allowed {
            methods.push_str(method.as_str());
            methods.push(',');
        }
        methods.pop();

        res.headers_mut().insert(ALLOW, methods.parse().unwrap());

        *res.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        async { res }
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{error::BodyError, request::WebRequest};

    use super::*;

    #[test]
    fn extract_default_impls() {
        let mut req = WebRequest::new_test(());
        let req = req.as_web_req();

        Option::<()>::from_request(&req).now_or_panic().unwrap().unwrap();

        Result::<(), ExtractError<BodyError>>::from_request(&req)
            .now_or_panic()
            .unwrap()
            .unwrap();

        <&WebRequest<'_>>::from_request(&req).now_or_panic().unwrap();

        <()>::from_request(&req).now_or_panic().unwrap();
    }
}
