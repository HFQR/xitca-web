use http::{header::HeaderMap, Method};

use crate::util::date::Date;

/// Context is connection specific struct contain states for processing.
/// It needs manually reset with every new successfully decoded request.
/// See `Context::reset` method for detail.
pub(super) struct Context<'a> {
    /// user configuration of keep alive.
    enable_ka: bool,
    /// set to true when connection has 100-continue header.
    is_expect: bool,
    ctype: ConnectionType,
    /// method cache of current request. Used for generate correct response.
    pub(super) req_method: Method,
    /// header map reused by next request.
    pub(super) header_cache: Option<HeaderMap>,
    /// smart pointer of cached date with 1 second update interval.
    pub(super) date: &'a Date,
}

impl<'a> Context<'a> {
    /// No particular reason. Copied from `actix-http` crate.
    pub(super) const MAX_HEADERS: usize = 96;

    pub(super) fn new(date: &'a Date) -> Self {
        Self {
            enable_ka: true,
            is_expect: false,
            ctype: ConnectionType::KeepAlive,
            req_method: Method::default(),
            header_cache: None,
            date,
        }
    }

    #[inline(always)]
    pub(super) fn is_expect(&self) -> bool {
        self.is_expect
    }

    #[inline(always)]
    pub(super) fn is_keep_alive(&self) -> bool {
        matches!(self.ctype, ConnectionType::KeepAlive)
    }

    #[inline(always)]
    pub(super) fn req_method(&self) -> &Method {
        &self.req_method
    }

    /// Context should be reset when a new request is decoded.
    #[inline(always)]
    pub(super) fn reset(&mut self) {
        self.ctype = ConnectionType::KeepAlive;
        self.is_expect = false;
    }

    pub(super) fn set_expect(&mut self) {
        self.is_expect = true;
    }

    #[inline(always)]
    pub(super) fn set_method(&mut self, method: Method) {
        self.req_method = method;
    }

    pub(super) fn set_ctype(&mut self, ctype: ConnectionType) {
        match (self.ctype, ctype) {
            // When connection is in upgrade state it can not be override.
            (ConnectionType::Upgrade, _) => {}
            // only set connection to keep alive when enabled.
            (_, ConnectionType::KeepAlive) if self.enable_ka => self.ctype = ConnectionType::KeepAlive,
            _ => self.ctype = ctype,
        }
    }

    #[inline(always)]
    pub(super) fn ctype(&self) -> ConnectionType {
        self.ctype
    }
}

/// Represents various types of connection
#[derive(Copy, Clone, PartialEq, Debug)]
pub(super) enum ConnectionType {
    /// Close connection after response
    Close,

    /// Keep connection alive after response
    KeepAlive,

    /// Connection is upgraded to different type
    Upgrade,
}
