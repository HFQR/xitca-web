use http::{header::HeaderMap, Method};

use crate::util::date::Date;

pub(super) struct Context<'a> {
    pub(super) flag: ContextFlag,
    pub(super) ctype: ConnectionType,
    pub(super) method: Method,
    pub(super) header_cache: Option<HeaderMap>,
    pub(super) date: &'a Date,
}

impl<'a> Context<'a> {
    pub(super) const MAX_HEADERS: usize = 96;

    pub(super) fn new(date: &'a Date) -> Self {
        let flag = ContextFlag::new(false);

        Self {
            flag,
            ctype: ConnectionType::Close,
            method: Method::default(),
            header_cache: None,
            date,
        }
    }
}

pub(super) struct ContextFlag {
    flag: u8,
}

impl ContextFlag {
    const ENABLE_KEEP_ALIVE: u8 = 0b0_01;

    fn new(enable_ka: bool) -> Self {
        let flag = if enable_ka { Self::ENABLE_KEEP_ALIVE } else { 0 };

        Self { flag }
    }

    pub(super) fn keep_alive_enable(&self) -> bool {
        self.flag & Self::ENABLE_KEEP_ALIVE == Self::ENABLE_KEEP_ALIVE
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
