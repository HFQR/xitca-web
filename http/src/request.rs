use std::{
    net::SocketAddr,
    ops::{Deref, DerefMut},
};

use super::http;

/// Extened request type for xitca-http.
///
/// It extends on [http::Rquest] type with additional fields that it does not provided.
/// It dereference to [http::Request].
pub struct Request<B> {
    req: http::Request<B>,
    remote_addr: Option<SocketAddr>,
}

impl<B> Request<B> {
    #[inline(always)]
    pub fn new(body: B) -> Self {
        Self::with_remote_addr(body, None)
    }

    #[inline]
    pub fn with_remote_addr(body: B, remote_addr: Option<SocketAddr>) -> Self {
        Self {
            req: http::Request::new(body),
            remote_addr,
        }
    }

    /// Construct from [http::Request]
    #[inline(always)]
    pub fn from_http(req: http::Request<B>, remote_addr: Option<SocketAddr>) -> Self {
        Self { req, remote_addr }
    }

    /// Get remote socket address of this request's source.
    #[inline]
    pub fn remote_addr(&self) -> Option<&SocketAddr> {
        self.remote_addr.as_ref()
    }

    pub fn map_body<F, B1>(self, func: F) -> Request<B1>
    where
        F: FnOnce(B) -> B1,
    {
        let Self { req, remote_addr } = self;

        let (parts, body) = req.into_parts();
        let req = http::Request::from_parts(parts, func(body));

        Request { req, remote_addr }
    }

    pub fn replace_body<B1>(self, b1: B1) -> (Request<B1>, B) {
        let Self { req, remote_addr } = self;

        let (parts, b) = req.into_parts();
        let req = http::Request::from_parts(parts, b1);

        (Request { req, remote_addr }, b)
    }

    /// Forward to [http::Request::into_body]
    #[inline(always)]
    pub fn into_body(self) -> B {
        self.req.into_body()
    }

    /// Forward to [http::Request::into_parts].
    #[inline(always)]
    pub fn into_parts(self) -> (http::request::Parts, B) {
        self.req.into_parts()
    }
}

impl<B> Default for Request<B>
where
    B: Default,
{
    fn default() -> Self {
        Self::new(B::default())
    }
}

impl<B> Deref for Request<B> {
    type Target = http::Request<B>;

    fn deref(&self) -> &Self::Target {
        &self.req
    }
}

impl<B> DerefMut for Request<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.req
    }
}
