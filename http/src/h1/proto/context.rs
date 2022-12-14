use std::{mem, net::SocketAddr};

use crate::{
    date::DateTime,
    http::{header::HeaderMap, Extensions},
};

/// Context is connection specific struct contain states for processing.
pub struct Context<'a, D, const HEADER_LIMIT: usize> {
    addr: SocketAddr,
    state: ContextState,
    ctype: ConnectionType,
    // header map reused by next request.
    header: Option<HeaderMap>,
    // http extensions reused by next request.
    exts: Extensions,
    pub(super) date: &'a D,
}

/// A set of state for current request that are used after request's ownership is passed
/// to service call.
struct ContextState(u8);

impl ContextState {
    /// Enable when current request has 100-continue header.
    const EXPECT: u8 = 0b_0001;

    /// Enable when current request is CONNECT method.
    const CONNECT: u8 = 0b_0010;

    /// Enable when current request is HEAD method.
    const HEAD: u8 = 0b_0100;

    const fn new() -> Self {
        Self(0)
    }

    fn insert(&mut self, other: u8) {
        self.0 |= other;
    }

    const fn contains(&self, other: u8) -> bool {
        (self.0 & other) == other
    }
}

/// Represents various types of connection
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ConnectionType {
    /// A connection that has no request yet.
    Init,

    /// Close connection after response with flush and shutdown IO.
    Close,

    /// Keep connection alive after response
    KeepAlive,
}

impl<'a, D, const HEADER_LIMIT: usize> Context<'a, D, HEADER_LIMIT> {
    /// Context is constructed with reference of certain type that impl [DateTime] trait.
    #[inline]
    pub fn new(date: &'a D) -> Self
    where
        D: DateTime,
    {
        Self::with_addr(crate::unspecified_socket_addr(), date)
    }

    /// Context is constructed with [SocketAddr] and reference of certain type that impl [DateTime] trait.
    #[inline]
    pub fn with_addr(addr: SocketAddr, date: &'a D) -> Self
    where
        D: DateTime,
    {
        Self {
            addr,
            state: ContextState::new(),
            ctype: ConnectionType::Init,
            header: None,
            exts: Extensions::new(),
            date,
        }
    }

    /// Take ownership of HeaderMap stored in Context.
    ///
    /// When Context does not have one a new HeaderMap is constructed.
    #[inline]
    pub fn take_headers(&mut self) -> HeaderMap {
        self.header.take().unwrap_or_else(HeaderMap::new)
    }

    /// Take ownership of Extensions stored in Context.
    #[inline]
    pub fn take_extensions(&mut self) -> Extensions {
        mem::take(&mut self.exts)
    }

    /// Replace a new HeaderMap in current Context.
    #[inline]
    pub fn replace_headers(&mut self, headers: HeaderMap) {
        debug_assert!(headers.is_empty());
        self.header = Some(headers);
    }

    /// Replace a new Extensions in current Context.
    #[inline]
    pub fn replace_extensions(&mut self, extensions: Extensions) {
        debug_assert!(extensions.is_empty());
        self.exts = extensions;
    }

    /// Reset Context's state to partial default state.
    #[inline]
    pub fn reset(&mut self) {
        self.ctype = ConnectionType::KeepAlive;
        self.state = ContextState::new();
    }

    /// Set Context's state to EXPECT header received.
    #[inline]
    pub fn set_expect_header(&mut self) {
        self.state.insert(ContextState::EXPECT)
    }

    /// Set Context's state to CONNECT method received.
    #[inline]
    pub fn set_connect_method(&mut self) {
        self.state.insert(ContextState::CONNECT)
    }

    /// Set Context's state to HEAD method received.
    #[inline]
    pub fn set_head_method(&mut self) {
        self.state.insert(ContextState::HEAD)
    }

    /// Set connection type.
    #[inline]
    pub fn set_ctype(&mut self, ctype: ConnectionType) {
        self.ctype = ctype;
    }

    /// Get expect header state.
    #[inline]
    pub fn is_expect_header(&self) -> bool {
        self.state.contains(ContextState::EXPECT)
    }

    /// Get CONNECT method state.
    #[inline]
    pub fn is_connect_method(&self) -> bool {
        self.state.contains(ContextState::CONNECT)
    }

    /// Get HEAD method state.
    #[inline]
    pub fn is_head_method(&self) -> bool {
        self.state.contains(ContextState::HEAD)
    }

    /// Return true if connection type is [ConnectionType::Close] or [ConnectionType::CloseForce].
    #[inline]
    pub fn is_connection_closed(&self) -> bool {
        matches!(self.ctype, ConnectionType::Close)
    }

    /// Get connection type.
    #[inline]
    pub fn ctype(&self) -> ConnectionType {
        self.ctype
    }

    /// Get remote socket address context associated with.
    #[inline]
    pub fn socket_addr(&self) -> &SocketAddr {
        &self.addr
    }
}
