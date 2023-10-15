use core::convert::Infallible;

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

impl<'r, C, B, T, E> FromRequest<WebRequest<'r, C, B>> for Result<T, E>
where
    B: BodyStream,
    T: FromRequest<WebRequest<'r, C, B>, Error = E>,
{
    type Type<'b> = Result<T::Type<'b>, E>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request<'a>(req: &'a WebRequest<'r, C, B>) -> Result<Self::Type<'a>, Self::Error> {
        Ok(T::from_request(req).await)
    }
}

impl<'r, C, B, T> FromRequest<WebRequest<'r, C, B>> for Option<T>
where
    B: BodyStream,
    T: FromRequest<WebRequest<'r, C, B>>,
{
    type Type<'b> = Option<T::Type<'b>>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request<'a>(req: &'a WebRequest<'r, C, B>) -> Result<Self::Type<'a>, Self::Error> {
        Ok(T::from_request(req).await.ok())
    }
}

impl<'r, C, B> FromRequest<WebRequest<'r, C, B>> for &WebRequest<'_, C, B>
where
    C: 'static,
    B: BodyStream + 'static,
{
    type Type<'b> = &'b WebRequest<'b, C, B>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request<'a>(req: &'a WebRequest<'r, C, B>) -> Result<Self::Type<'a>, Self::Error> {
        Ok(req)
    }
}

impl<'r, C, B> FromRequest<WebRequest<'r, C, B>> for ()
where
    B: BodyStream,
{
    type Type<'b> = ();
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request<'a>(_: &'a WebRequest<'r, C, B>) -> Result<Self::Type<'a>, Self::Error> {
        Ok(())
    }
}

impl<'r, C, B, ResB> Responder<WebRequest<'r, C, B>> for WebResponse<ResB> {
    type Output = WebResponse<ResB>;

    #[inline]
    async fn respond_to(self, _: WebRequest<'r, C, B>) -> Self::Output {
        self
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for () {
    type Output = WebResponse;

    #[inline]
    async fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Output {
        req.into_response(Bytes::new())
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for Infallible {
    type Output = WebResponse;

    async fn respond_to(self, _: WebRequest<'r, C, B>) -> Self::Output {
        match self {}
    }
}

macro_rules! text_utf8 {
    ($type: ty) => {
        impl<'r, C, B> Responder<WebRequest<'r, C, B>> for $type {
            type Output = WebResponse;

            async fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Output {
                let mut res = req.into_response(self);
                res.headers_mut().insert(CONTENT_TYPE, TEXT_UTF8);
                res
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

            async fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Output {
                let mut res = req.into_response(Bytes::new());
                *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                res
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

    async fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Output {
        let mut res = req.into_response(Bytes::new());
        *res.status_mut() = StatusCode::NOT_FOUND;
        res
    }
}

impl<'r, C, B> Responder<WebRequest<'r, C, B>> for MethodNotAllowed {
    type Output = WebResponse;

    async fn respond_to(self, req: WebRequest<'r, C, B>) -> Self::Output {
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

        res
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
