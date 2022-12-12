use std::{
    borrow::{Borrow, BorrowMut},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    ops::{Deref, DerefMut},
};

use super::http;

// This type has 12 less bytes in size compared to SocketAddr which can ultimately effect
// if Request<B> would be inlined or copied when passing around as Service::call argument.
/// A simplified version of [SocketAddr] where only [IpAddr] and `Port` information is stored.
#[derive(Debug, Copy, Clone)]
pub enum RemoteAddr {
    V4(Ipv4Addr, u16),
    V6(Ipv6Addr, u16),
}

impl Default for RemoteAddr {
    fn default() -> Self {
        Self::V4(Ipv4Addr::UNSPECIFIED, 0)
    }
}

impl From<SocketAddr> for RemoteAddr {
    fn from(addr: SocketAddr) -> Self {
        match addr {
            SocketAddr::V4(v4) => Self::V4(*v4.ip(), v4.port()),
            SocketAddr::V6(v6) => Self::V6(*v6.ip(), v6.port()),
        }
    }
}

/// Extended request type for xitca-http.
///
/// It extends on [http::Request] type with additional state.
///
/// Request impls [Borrow]/[BorrowMut]/[Deref]/[DerefMut] trait and they can be used to
/// get a direct reference to [http::Request] type.
pub struct Request<B> {
    req: http::Request<B>,
    addr: RemoteAddr,
}

impl<B> Request<B> {
    #[inline(always)]
    pub fn new(body: B) -> Self {
        Self::with_remote_addr(body, Default::default())
    }

    #[inline]
    pub fn with_remote_addr(body: B, addr: RemoteAddr) -> Self {
        Self::from_http(http::Request::new(body), addr)
    }

    /// Construct from existing [http::Request] and [RemoteAddr].
    #[inline(always)]
    pub fn from_http(req: http::Request<B>, addr: RemoteAddr) -> Self {
        Self { req, addr }
    }

    /// Get remote socket address of this request's source.
    #[inline]
    pub fn remote_addr(&self) -> &RemoteAddr {
        &self.addr
    }

    #[inline]
    pub fn map_body<F, B1>(self, func: F) -> Request<B1>
    where
        F: FnOnce(B) -> B1,
    {
        Request {
            req: self.req.map(func),
            addr: self.addr,
        }
    }

    #[inline]
    pub fn replace_body<B1>(self, b1: B1) -> (Request<B1>, B) {
        let (parts, b) = self.req.into_parts();
        let req = http::Request::from_parts(parts, b1);

        (Request { req, addr: self.addr }, b)
    }

    /// Forward to [http::Request::into_body]
    #[inline]
    pub fn into_body(self) -> B {
        self.req.into_body()
    }

    /// Forward to [http::Request::into_parts].
    #[inline]
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

impl<B> Borrow<http::Request<B>> for Request<B> {
    fn borrow(&self) -> &http::Request<B> {
        &self.req
    }
}

impl<B> BorrowMut<http::Request<B>> for Request<B> {
    fn borrow_mut(&mut self) -> &mut http::Request<B> {
        &mut self.req
    }
}

/// trait for Borrow &T from &Self.
/// used for foreign types that can be impl with [Borrow] trait.
pub trait BorrowReq<T> {
    fn borrow(&self) -> &T;
}

/// trait for Borrow &mut T from &mut Self.
/// used for foreign types that can be impl with [BorrowMut] trait.
pub trait BorrowReqMut<T> {
    fn borrow_mut(&mut self) -> &mut T;
}

impl<B> BorrowReq<http::Uri> for http::Request<B> {
    fn borrow(&self) -> &http::Uri {
        self.uri()
    }
}

impl<B> BorrowReq<http::Method> for http::Request<B> {
    fn borrow(&self) -> &http::Method {
        self.method()
    }
}

impl<B> BorrowReqMut<http::Extensions> for http::Request<B> {
    fn borrow_mut(&mut self) -> &mut http::Extensions {
        self.extensions_mut()
    }
}

impl<B, T> BorrowReq<T> for Request<B>
where
    http::Request<B>: BorrowReq<T>,
{
    fn borrow(&self) -> &T {
        <http::Request<B> as BorrowReq<T>>::borrow(&self.req)
    }
}

impl<B, T> BorrowReqMut<T> for Request<B>
where
    http::Request<B>: BorrowReqMut<T>,
{
    fn borrow_mut(&mut self) -> &mut T {
        <http::Request<B> as BorrowReqMut<T>>::borrow_mut(&mut self.req)
    }
}
