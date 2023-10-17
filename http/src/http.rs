//! re-export of [http] crate types.

pub use ::http::*;

use core::{
    borrow::{Borrow, BorrowMut},
    pin::Pin,
    task::{Context, Poll},
};

use std::net::SocketAddr;

use futures_core::stream::Stream;
use pin_project_lite::pin_project;

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

impl<ReqB, B, ResB> IntoResponse<B, ResB> for Request<ReqB>
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

#[cfg(feature = "router")]
use super::util::service::router::Params;

pin_project! {
    /// typed http extension
    #[derive(Debug)]
    pub struct RequestExt<B> {
        #[pin]
        body: B,
        ext: Extension,
    }
}

// a separate extension type contain information can not be carried by http::Request. the goal is
// to keep extended info strongly typed and not depend on runtime type map of http::Extensions.
#[derive(Debug)]
pub(crate) struct Extension(Box<_Extension>);

impl Extension {
    pub(crate) fn new(addr: SocketAddr) -> Self {
        Self(Box::new(_Extension {
            addr,
            #[cfg(feature = "router")]
            params: Default::default(),
        }))
    }
}

#[derive(Debug)]
struct _Extension {
    addr: SocketAddr,
    #[cfg(feature = "router")]
    params: Params,
}

impl<B> RequestExt<B> {
    pub(crate) fn from_parts(body: B, ext: Extension) -> Self {
        Self { body, ext }
    }

    #[inline]
    pub fn socket_addr(&self) -> &SocketAddr {
        &self.ext.0.addr
    }

    #[inline]
    pub fn socket_addr_mut(&mut self) -> &mut SocketAddr {
        &mut self.ext.0.addr
    }

    #[inline]
    pub fn map_body<F, B1>(self, func: F) -> RequestExt<B1>
    where
        F: FnOnce(B) -> B1,
    {
        RequestExt {
            body: func(self.body),
            ext: self.ext,
        }
    }

    #[inline]
    pub fn replace_body<B1>(self, body: B1) -> (RequestExt<B1>, B) {
        let body_org = self.body;

        (RequestExt { body, ext: self.ext }, body_org)
    }
}

#[cfg(feature = "router")]
impl<B> RequestExt<B> {
    #[inline]
    pub fn params(&self) -> &Params {
        &self.ext.0.params
    }

    #[inline]
    pub fn params_mut(&mut self) -> &mut Params {
        &mut self.ext.0.params
    }
}

impl<B> Default for RequestExt<B>
where
    B: Default,
{
    fn default() -> Self {
        Self::from_parts(B::default(), Extension::new(crate::unspecified_socket_addr()))
    }
}

impl<B> Stream for RequestExt<B>
where
    B: Stream,
{
    type Item = B::Item;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().body.poll_next(cx)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.body.size_hint()
    }
}

impl<B> Borrow<SocketAddr> for RequestExt<B> {
    #[inline]
    fn borrow(&self) -> &SocketAddr {
        self.socket_addr()
    }
}

#[cfg(feature = "router")]
impl<B> Borrow<Params> for RequestExt<B> {
    #[inline]
    fn borrow(&self) -> &Params {
        self.params()
    }
}

#[cfg(feature = "router")]
impl<B> BorrowMut<Params> for RequestExt<B> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut Params {
        self.params_mut()
    }
}

/// trait for Borrow &T from &Self.
/// used for foreign types (from xitca-http pov) that can be impl with [Borrow] trait.
pub trait BorrowReq<T> {
    fn borrow(&self) -> &T;
}

/// trait for Borrow &mut T from &mut Self.
/// used for foreign types (from xitca-http pov) that can be impl with [BorrowMut] trait.
pub trait BorrowReqMut<T> {
    fn borrow_mut(&mut self) -> &mut T;
}

impl<Ext> BorrowReq<Uri> for Request<Ext> {
    #[inline]
    fn borrow(&self) -> &Uri {
        self.uri()
    }
}

impl<Ext> BorrowReq<Method> for Request<Ext> {
    #[inline]
    fn borrow(&self) -> &Method {
        self.method()
    }
}

impl<Ext> BorrowReqMut<Extensions> for Request<Ext> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut Extensions {
        self.extensions_mut()
    }
}

impl<T, B> BorrowReq<T> for Request<RequestExt<B>>
where
    RequestExt<B>: Borrow<T>,
{
    #[inline]
    fn borrow(&self) -> &T {
        self.body().borrow()
    }
}

impl<T, B> BorrowReqMut<T> for Request<RequestExt<B>>
where
    RequestExt<B>: BorrowMut<T>,
{
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self.body_mut().borrow_mut()
    }
}
