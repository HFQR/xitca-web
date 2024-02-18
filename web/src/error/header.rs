use core::fmt;

use std::error;

use crate::http::HeaderName;

use super::{error_from_service, forward_blank_bad_request};

/// error type when named header is not found from request.
#[derive(Debug)]
pub struct HeaderNotFound(pub HeaderName);

impl fmt::Display for HeaderNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HeaderName: {} is not found", self.0.as_str())
    }
}

impl error::Error for HeaderNotFound {}

error_from_service!(HeaderNotFound);
forward_blank_bad_request!(HeaderNotFound);

/// error type when named header is not associated with valid header value.
#[derive(Debug)]
pub struct InvalidHeaderValue(pub HeaderName);

impl fmt::Display for InvalidHeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HeaderName: {} associated with invalid HeaderValue", self.0.as_str())
    }
}

impl error::Error for InvalidHeaderValue {}

error_from_service!(InvalidHeaderValue);
forward_blank_bad_request!(InvalidHeaderValue);
