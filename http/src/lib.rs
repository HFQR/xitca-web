//! Http module for [Service](xitca_service::Service) trait oriented http handling.
//!
//! This crate tries to serve both low overhead and ease of use purpose.
//! All http protocols can be used separately with corresponding feature flag or work together
//! for handling different protocols in one place.

#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

mod builder;
mod request;
mod response;
mod service;
mod tls;
mod version;

pub mod body;
pub mod date;
pub mod error;

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

    use crate::body::ResponseBody;

    /// Some often used header value.
    #[allow(clippy::declare_interior_mutable_const)]
    pub mod const_header_value {
        use super::*;

        pub const TEXT_UTF8: HeaderValue = HeaderValue::from_static("text/plain; charset=utf-8");
        pub const JSON: HeaderValue = HeaderValue::from_static("application/json");
    }

    /// Helper trait for convert a [Request] to [Response].
    /// This is for re-use request's heap allocation and pass down the context data inside [Extensions]
    pub trait IntoResponse<B, ResB> {
        fn into_response(self, body: B) -> Response<ResponseBody<ResB>>;

        fn as_response(&mut self, body: B) -> Response<ResponseBody<ResB>>
        where
            Self: Default,
        {
            std::mem::take(self).into_response(body)
        }
    }

    impl<ReqB, B, ResB> IntoResponse<B, ResB> for super::request::Request<ReqB>
    where
        B: Into<ResponseBody<ResB>>,
    {
        fn into_response(self, body: B) -> Response<ResponseBody<ResB>> {
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
pub use builder::HttpServiceBuilder;
pub use error::{BodyError, HttpServiceError};
pub use request::Request;
pub use response::ResponseError;
pub use service::HttpService;

// TODO: enable this conflict feature check.
// temporary compile error for conflicted feature combination.
// #[cfg(not(feature = "http1"))]
// #[cfg(all(feature = "http2", feature = "native-tls"))]
// compile_error!("http2 feature can not use native-tls");
