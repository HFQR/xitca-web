use core::fmt;

use std::error;

use super::{error_from_service, forward_blank_bad_request};

pub use xitca_http::error::BodyError;

#[derive(Debug, Clone)]
pub struct BodyOverFlow {
    pub(crate) limit: usize,
}

impl fmt::Display for BodyOverFlow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "body size reached limit: {} bytes", self.limit)
    }
}

impl error::Error for BodyOverFlow {}

error_from_service!(BodyOverFlow);
forward_blank_bad_request!(BodyOverFlow);
