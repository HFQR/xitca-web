//! Http module for [Service](xitca_service::Service) trait oriented http handling.
//!
//! This crate tries to serve both low overhead and ease of use purpose.
//! All http protocols can be used separately with corresponding feature flag or work together
//! for handling different protocols in one place.

#![forbid(unsafe_code)]
#![feature(type_alias_impl_trait)]

#[cfg(feature = "runtime")]
mod builder;
#[cfg(feature = "runtime")]
mod service;
mod tls;
mod version;

pub mod body;
pub mod error;
pub mod request;
pub mod response;

#[cfg(feature = "runtime")]
pub mod date;
#[cfg(feature = "http1")]
pub mod h1;
#[cfg(feature = "http2")]
pub mod h2;
#[cfg(feature = "http3")]
pub mod h3;

pub mod config;
pub mod util;

/// re-export http crate as module.
pub mod http {
    pub use http::*;

    use std::ops::{Deref, DerefMut};

    use xitca_unsafe_collection::bytes::BytesStr;

    pub struct Params(Vec<(BytesStr, String)>);

    impl Default for Params {
        fn default() -> Self {
            Self::with_capacity(0)
        }
    }

    impl Params {
        #[inline]
        pub fn with_capacity(cap: usize) -> Self {
            Self(Vec::with_capacity(cap))
        }

        #[inline]
        pub fn insert(&mut self, key: BytesStr, value: String) {
            self.0.push((key, value))
        }

        pub fn get(&self, key: &str) -> Option<&str> {
            self.0.iter().find_map(|(k, v)| k.eq(key).then_some(v.as_str()))
        }

        pub fn remove(&mut self, key: &str) -> Option<(BytesStr, String)> {
            self.0
                .iter()
                .enumerate()
                .find_map(|(i, v)| v.0.eq(key).then_some(i))
                .map(|i| self.0.swap_remove(i))
        }
    }

    impl Deref for Params {
        type Target = Vec<(BytesStr, String)>;

        #[inline]
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl DerefMut for Params {
        #[inline]
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }

    /// Some often used header value.
    #[allow(clippy::declare_interior_mutable_const)]
    pub mod const_header_value {
        use ::http::header::HeaderValue;

        macro_rules! const_value {
            ($(($ident: ident, $expr: expr)), *) => {
                $(
                   pub const $ident: HeaderValue = HeaderValue::from_static($expr);
                )*
            }
        }

        const_value!(
            (TEXT, "text/plain"),
            (TEXT_UTF8, "text/plain; charset=utf-8"),
            (JSON, "application/json"),
            (TEXT_HTML_UTF8, "text/html; charset=utf-8"),
            (GRPC, "application/grpc"),
            (WEBSOCKET, "websocket")
        );
    }

    /// Some often used header name.
    #[allow(clippy::declare_interior_mutable_const)]
    pub mod const_header_name {
        use ::http::header::HeaderName;

        macro_rules! const_name {
            ($(($ident: ident, $expr: expr)), *) => {
                $(
                   pub const $ident: HeaderName = HeaderName::from_static($expr);
                )*
            }
        }

        const_name!((PROTOCOL, "protocol"));
    }

    /// Helper trait for convert a [Request] to [Response].
    /// This is for re-use request's heap allocation and pass down the context data inside [Extensions]
    pub trait IntoResponse<B, ResB> {
        fn into_response(self, body: B) -> Response<ResB>;

        fn as_response(&mut self, body: B) -> Response<ResB>
        where
            Self: Default,
        {
            std::mem::take(self).into_response(body)
        }
    }

    impl<ReqB, B, ResB> IntoResponse<B, ResB> for super::request::Request<ReqB>
    where
        B: Into<ResB>,
    {
        fn into_response(self, body: B) -> Response<ResB> {
            let (
                request::Parts {
                    mut headers,
                    extensions,
                    ..
                },
                _,
            ) = self.into_parts();
            headers.clear();

            let mut res = Response::new(body.into());
            *res.headers_mut() = headers;
            *res.extensions_mut() = extensions;

            res
        }
    }
}

/// re-export bytes crate as module.
pub use xitca_io::bytes;

pub use body::{RequestBody, ResponseBody};
#[cfg(feature = "runtime")]
pub use builder::HttpServiceBuilder;
pub use error::{BodyError, HttpServiceError};
pub use request::Request;
pub use response::Response;
#[cfg(feature = "runtime")]
pub use service::HttpService;

// TODO: enable this conflict feature check.
// temporary compile error for conflicted feature combination.
// #[cfg(not(feature = "http1"))]
// #[cfg(all(feature = "http2", feature = "native-tls"))]
// compile_error!("http2 feature can not use native-tls");
