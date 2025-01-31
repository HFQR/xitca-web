//! type extractor for header value

use core::{fmt, ops::Deref};

use crate::{
    body::ResponseBody,
    context::WebContext,
    error::{Error, HeaderNotFound},
    handler::{FromRequest, Responder},
    http::{
        WebResponse,
        header::{self, HeaderMap, HeaderName, HeaderValue},
    },
};

macro_rules! const_header_name {
    ($n:expr ;) => {};
    ($n:expr ; $i: ident $(, $rest:ident)*) => {
        pub const $i: usize = $n;
        const_header_name!($n + 1; $($rest),*);
    };
    ($($i:ident), +) => { const_header_name!(0; $($i),*); };
}

macro_rules! map_to_header_name {
    ($($i:ident), +) => {
        const fn map_to_header_name<const HEADER_NAME: usize>() -> header::HeaderName {
            match HEADER_NAME  {
            $(
                $i => header::$i,
            )*
                _ => unreachable!()
            }
        }
    }
}

macro_rules! const_header_name_impl {
    ($($i:ident), +) => {
        const_header_name!($($i), +);
        map_to_header_name!($($i), +);
    }
}

const_header_name_impl!(ACCEPT, ACCEPT_ENCODING, HOST, CONTENT_TYPE, CONTENT_LENGTH);

/// typed header extractor.
///
/// on success HeaderRef will be received in handler function where it can be dereference to
/// [HeaderValue] type.
///
/// on failure [HeaderNotFound] error would be returned which would generate a "400 BadRequest"
/// http response.
///
/// # Example
/// ```rust
/// use xitca_web::{
///     handler::{
///         handler_service,
///         header::{self, HeaderRef}
///     },
///     App
/// };
/// # use xitca_web::WebContext;
///
/// // a handle function expecting content_type header.
/// async fn handle(header: HeaderRef<'_, { header::CONTENT_TYPE }>) -> &'static str {
///     // dereference HeaderRef to operate on HeaderValue.
///     println!("{:?}", header.to_str());
///     ""
/// }
///
/// App::new()
///     .at("/", handler_service(handle))
///     # .at("/nah", handler_service(|_: &WebContext<'_>| async { "for type infer" }));
/// ```
pub struct HeaderRef<'a, const HEADER_NAME: usize>(&'a HeaderValue);

impl<const HEADER_NAME: usize> fmt::Debug for HeaderRef<'_, HEADER_NAME> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Header")
            .field("name", &map_to_header_name::<HEADER_NAME>())
            .field("value", &self.0)
            .finish()
    }
}

impl<const HEADER_NAME: usize> Deref for HeaderRef<'_, HEADER_NAME> {
    type Target = HeaderValue;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B, const HEADER_NAME: usize> FromRequest<'a, WebContext<'r, C, B>> for HeaderRef<'a, HEADER_NAME> {
    type Type<'b> = HeaderRef<'b, HEADER_NAME>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let name = map_to_header_name::<HEADER_NAME>();
        ctx.req()
            .headers()
            .get(&name)
            .map(HeaderRef)
            .ok_or_else(|| Error::from_service(HeaderNotFound(name)))
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for (HeaderName, HeaderValue) {
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = ctx.into_response(ResponseBody::empty());
        Responder::<WebContext<'r, C, B>>::map(self, res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        res.headers_mut().append(self.0, self.1);
        Ok(res)
    }
}

impl<'r, C, B, const N: usize> Responder<WebContext<'r, C, B>> for [(HeaderName, HeaderValue); N] {
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = ctx.into_response(ResponseBody::empty());
        Responder::<WebContext<'r, C, B>>::map(self, res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        for (k, v) in self {
            res.headers_mut().append(k, v);
        }
        Ok(res)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for Vec<(HeaderName, HeaderValue)> {
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = ctx.into_response(ResponseBody::empty());
        Responder::<WebContext<'r, C, B>>::map(self, res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        for (k, v) in self {
            res.headers_mut().append(k, v);
        }
        Ok(res)
    }
}

impl<'r, C, B> Responder<WebContext<'r, C, B>> for HeaderMap {
    type Response = WebResponse;
    type Error = Error;

    async fn respond(self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let res = ctx.into_response(ResponseBody::empty());
        Responder::<WebContext<'r, C, B>>::map(self, res)
    }

    fn map(self, mut res: Self::Response) -> Result<Self::Response, Self::Error> {
        res.headers_mut().extend(self);
        Ok(res)
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use super::*;

    #[test]
    fn extract_header() {
        let mut req = WebContext::new_test(());
        let mut req = req.as_web_ctx();
        req.req_mut()
            .headers_mut()
            .insert(header::HOST, header::HeaderValue::from_static("996"));
        req.req_mut()
            .headers_mut()
            .insert(header::ACCEPT_ENCODING, header::HeaderValue::from_static("251"));

        assert_eq!(
            HeaderRef::<'_, { super::ACCEPT_ENCODING }>::from_request(&req)
                .now_or_panic()
                .unwrap()
                .deref(),
            &header::HeaderValue::from_static("251")
        );
        assert_eq!(
            HeaderRef::<'_, { super::HOST }>::from_request(&req)
                .now_or_panic()
                .unwrap()
                .deref(),
            &header::HeaderValue::from_static("996")
        );
    }
}
