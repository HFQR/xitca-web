use std::cell::{Ref, RefCell, RefMut};

use actix_http_alt::{http::Request, RequestBody};

pub struct WebRequest<'a, D> {
    pub(crate) http: RefCell<Request<RequestBody>>,
    pub(crate) state: &'a D,
}

impl<'a, D> WebRequest<'a, D> {
    #[doc(hidden)]
    pub fn new(http: Request<RequestBody>, state: &'a D) -> Self {
        Self {
            http: RefCell::new(http),
            state,
        }
    }

    #[cfg(test)]
    pub fn with_state(state: &'a D) -> Self {
        Self {
            http: RefCell::new(Request::default()),
            state,
        }
    }

    /// Get an immutable reference of [HttpRequest](crate::request::HttpRequest)
    #[inline]
    pub fn request_ref(&self) -> Ref<'_, Request<RequestBody>> {
        self.http.borrow()
    }

    /// Get a mutable reference of [HttpRequest](crate::request::HttpRequest)
    #[inline]
    pub fn request_ref_mut(&self) -> RefMut<'_, Request<RequestBody>> {
        self.http.borrow_mut()
    }

    /// Get a mutable reference of [HttpRequest](crate::request::HttpRequest)
    /// This API takes &mut WebRequest so it bypass runtime borrow checker
    /// and therefore has zero runtime overhead.
    #[inline]
    pub fn request_mut(&mut self) -> &mut Request<RequestBody> {
        self.http.get_mut()
    }

    /// Get an immutable reference of App state
    #[inline]
    pub fn state(&self) -> &D {
        self.state
    }
}
