use http::{header::HeaderMap, Method};

use crate::util::date::Date;

/// Context is connection specific struct contain states for processing.
/// It needs manually reset with every new successfully decoded request.
/// See `Context::reset` method for detail.
pub(super) struct Context<'a> {
    state: ContextState,
    ctype: ConnectionType,
    /// method cache of current request. Used for generate correct response.
    pub(super) req_method: Method,
    /// header map reused by next request.
    pub(super) header_cache: Option<HeaderMap>,
    /// smart pointer of cached date with 500 milli second update interval.
    pub(super) date: &'a Date,
}

impl<'a> Context<'a> {
    /// No particular reason. Copied from `actix-http` crate.
    pub(super) const MAX_HEADERS: usize = 96;

    pub(super) fn new(date: &'a Date) -> Self {
        Self {
            state: ContextState::new(),
            ctype: ConnectionType::Init,
            req_method: Method::default(),
            header_cache: None,
            date,
        }
    }

    #[inline(always)]
    pub(super) fn is_expect(&self) -> bool {
        self.state.contains(ContextState::EXPECT)
    }

    #[inline(always)]
    pub(super) fn is_force_close(&self) -> bool {
        self.state.contains(ContextState::FORCE_CLOSE)
    }

    #[inline(always)]
    pub(super) fn req_method(&self) -> &Method {
        &self.req_method
    }

    /// Context should be reset when a new request is decoded.
    ///
    /// A reset of context only happen on a keep alive connection type.
    #[inline(always)]
    pub(super) fn reset(&mut self) {
        self.ctype = ConnectionType::KeepAlive;
        self.state = ContextState::new();
    }

    pub(super) fn set_expect(&mut self) {
        self.state.insert(ContextState::EXPECT)
    }

    pub(super) fn set_force_close(&mut self) {
        self.state.insert(ContextState::FORCE_CLOSE)
    }

    #[inline(always)]
    pub(super) fn set_method(&mut self, method: Method) {
        self.req_method = method;
    }

    #[inline(always)]
    pub(super) fn set_ctype(&mut self, ctype: ConnectionType) {
        self.ctype = ctype;
    }

    #[inline(always)]
    pub(super) fn ctype(&self) -> ConnectionType {
        self.ctype
    }
}

struct ContextState(u8);

impl ContextState {
    /// Enable when current request has 100-continue header.
    const EXPECT: Self = Self(0b_0001);
    /// Want a force close after current request served.
    ///
    /// This is for situation like partial read of request body. Which could leave artifact
    /// unread data in connection that can interfere with next request(If the connection is kept
    /// alive).
    const FORCE_CLOSE: Self = Self(0b_0010);

    #[inline(always)]
    const fn new() -> Self {
        Self(0)
    }

    #[inline(always)]
    fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[inline(always)]
    const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// Represents various types of connection
#[derive(Copy, Clone, PartialEq, Debug)]
pub(super) enum ConnectionType {
    /// A connection that has no request yet.
    Init,

    /// Close connection after response
    Close,

    /// Keep connection alive after response
    KeepAlive,

    /// Connection is upgraded to different type
    Upgrade,
}
