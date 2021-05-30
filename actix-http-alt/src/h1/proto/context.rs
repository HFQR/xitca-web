use http::{header::HeaderMap, Method};

use crate::util::date::Date;

pub(super) struct Context<'a> {
    enable_ka: bool,
    is_expect: bool,
    pub(super) ctype: ConnectionType,
    pub(super) method: Method,
    pub(super) header_cache: Option<HeaderMap>,
    pub(super) date: &'a Date,
}

impl<'a> Context<'a> {
    pub(super) const MAX_HEADERS: usize = 96;

    pub(super) fn new(date: &'a Date) -> Self {
        Self {
            enable_ka: true,
            is_expect: false,
            ctype: ConnectionType::KeepAlive,
            method: Method::default(),
            header_cache: None,
            date,
        }
    }

    #[inline(always)]
    pub(super) fn is_expect(&self) -> bool {
        self.is_expect
    }

    #[inline(always)]
    pub(super) fn is_head_method(&self) -> bool {
        self.method == Method::HEAD
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

    pub(super) fn set_method(&mut self, method: Method) {
        self.method = method;
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
