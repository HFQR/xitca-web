use core::ops::{Deref, DerefMut};

use xitca_http::h1::proto::context;

use crate::date::DateTimeHandle;

type Ctx<'c, 'd, const HEADER_LIMIT: usize> = context::Context<'c, DateTimeHandle<'d>, HEADER_LIMIT>;

// xitca_http::h1::proto::context::Context always want a `&'a T: DateTime` as state.
//
// xitca_http::date::DateTime is a foreign trait so it can not be implemented to a lifetimed foreign type inside xitca-client.
// (RwLock<DateTimeState>) in this case.
//
// The double lifetime is to go around this limitation. See xitca_client::date module for details.
pub(crate) struct Context<'c, 'd, const HEADER_LIMIT: usize>(Ctx<'c, 'd, HEADER_LIMIT>);

impl<'c, 'd, const HEADER_LIMIT: usize> Deref for Context<'c, 'd, HEADER_LIMIT> {
    type Target = Ctx<'c, 'd, HEADER_LIMIT>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const HEADER_LIMIT: usize> DerefMut for Context<'_, '_, HEADER_LIMIT> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'c, 'd, const HEADER_LIMIT: usize> Context<'c, 'd, HEADER_LIMIT> {
    pub(crate) fn new(date: &'c DateTimeHandle<'d>, is_tls: bool) -> Self {
        Self(context::Context::new(date, is_tls))
    }
}
