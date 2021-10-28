use std::mem;

use crate::{
    date::DateTime,
    http::{header::HeaderMap, Extensions},
};

/// Context is connection specific struct contain states for processing.
pub struct Context<'a, D, const HEADER_LIMIT: usize> {
    state: ContextState,
    ctype: ConnectionType,
    /// header map reused by next request.
    header: Option<HeaderMap>,
    /// extension reused by next request.
    extensions: Extensions,
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

    /// Want a force close after current request served.
    ///
    /// This is for situation like partial read of request body. Which could leave artifact
    /// unread data in connection that can interfere with next request(If the connection is kept
    /// alive).
    const FORCE_CLOSE: u8 = 0b_0100;

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
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ConnectionType {
    /// A connection that has no request yet.
    Init,

    /// Close connection after response
    Close,

    /// Keep connection alive after response
    KeepAlive,

    /// Connection is upgraded to different type
    Upgrade,
}

impl<'a, D, const HEADER_LIMIT: usize> Context<'a, D, HEADER_LIMIT> {
    /// Context is constructed with a reference of certain type that impl [DateTime] trait.
    /// This trait is used to write date header to request/response.
    #[inline]
    pub fn new(date: &'a D) -> Self
    where
        D: DateTime,
    {
        Self {
            state: ContextState::new(),
            ctype: ConnectionType::Init,
            header: None,
            extensions: Extensions::new(),
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

    /// Take ownership of Extension stored in Context.
    #[inline]
    pub fn take_extensions(&mut self) -> Extensions {
        mem::take(&mut self.extensions)
    }

    /// Replace a new HeaderMap in current Context.
    #[inline]
    pub fn replace_headers(&mut self, header: HeaderMap) {
        self.header = Some(header);
    }

    /// Replace a new Extensions in current Context.
    #[inline]
    pub fn replace_extensions(&mut self, extensions: Extensions) {
        self.extensions = extensions;
    }

    /// Reset Context's state to partial default state.
    #[inline]
    pub fn reset(&mut self) {
        self.ctype = ConnectionType::KeepAlive;
        self.state = ContextState::new();
    }

    /// Set Context's state to expect header received.
    #[inline]
    pub fn set_expect_header(&mut self) {
        self.state.insert(ContextState::EXPECT)
    }

    /// Set Context's state to connect method received.
    #[inline]
    pub fn set_connect_method(&mut self) {
        self.state.insert(ContextState::CONNECT)
    }

    /// Set Context's state to connection closed.
    #[inline]
    pub fn set_force_close(&mut self) {
        self.state.insert(ContextState::FORCE_CLOSE)
    }

    /// Set Context's connection type.
    #[inline]
    pub fn set_ctype(&mut self, ctype: ConnectionType) {
        self.ctype = ctype;
    }

    /// Get current expect header state.
    #[inline]
    pub fn is_expect_header(&self) -> bool {
        self.state.contains(ContextState::EXPECT)
    }

    /// Get current connect method state.
    #[inline]
    pub fn is_connect_method(&self) -> bool {
        self.state.contains(ContextState::CONNECT)
    }

    /// Get current connection close state.
    #[inline]
    pub fn is_force_close(&self) -> bool {
        self.state.contains(ContextState::FORCE_CLOSE)
    }

    /// Get current connection type.
    #[inline]
    pub fn ctype(&self) -> ConnectionType {
        self.ctype
    }
}
