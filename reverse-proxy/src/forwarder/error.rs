use std::{error::Error, fmt};

/// Errors that can result from using a connector service.
#[derive(Debug)]
pub enum ForwardError {
    /// Failed to build a request from origin
    UriError(xitca_web::http::Error),
}

impl fmt::Display for ForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UriError(_) => f.write_str("could not build request from origin"),
        }
    }
}

impl Error for ForwardError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UriError(err) => Some(err),
        }
    }
}

impl ForwardError {
    pub fn into_error_status(self) -> xitca_web::error::ErrorStatus {
        match self {
            Self::UriError(_) => xitca_web::error::ErrorStatus::bad_request(),
        }
    }
}
