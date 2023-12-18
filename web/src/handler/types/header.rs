//! type extractor for header value

use core::{convert::Infallible, fmt, ops::Deref};

use std::error;

use crate::{
    body::BodyStream,
    context::WebContext,
    error::{BadRequest, Error},
    handler::FromRequest,
    http::{
        header::{self, HeaderValue},
        WebResponse,
    },
    service::Service,
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

impl<'a, 'r, C, B, const HEADER_NAME: usize> FromRequest<'a, WebContext<'r, C, B>> for HeaderRef<'a, HEADER_NAME>
where
    B: BodyStream,
{
    type Type<'b> = HeaderRef<'b, HEADER_NAME>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        ctx.req()
            .headers()
            .get(&map_to_header_name::<HEADER_NAME>())
            .map(HeaderRef)
            .ok_or_else(|| Error::from_service(HeaderNotFound(map_to_header_name::<HEADER_NAME>())))
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

#[derive(Debug)]
pub struct HeaderNotFound(pub(crate) header::HeaderName);

impl fmt::Display for HeaderNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HeaderName: {} not found", self.0.as_str())
    }
}

impl error::Error for HeaderNotFound {}

impl<'r, C, B> Service<WebContext<'r, C, B>> for HeaderNotFound {
    type Response = WebResponse;
    type Error = Infallible;

    async fn call(&self, ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        BadRequest.call(ctx).await
    }
}
