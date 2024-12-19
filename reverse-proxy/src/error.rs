use crate::forwarder::ForwardError;
use std::error::Error;
use std::fmt;
use xitca_web::error::Error as XitcaError;

#[derive(Debug)]
pub enum ProxyError {
    CannotReadRequestBody(XitcaError),
    ForwardError(ForwardError),
    NoPeer,
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CannotReadRequestBody(e) => write!(f, "error when reading request body: {}", e),
            Self::ForwardError(e) => write!(f, "error when forwarding request: {}", e),
            Self::NoPeer => f.write_str("no peer found"),
        }
    }
}

impl Error for ProxyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CannotReadRequestBody(err) => Some(err),
            Self::ForwardError(err) => Some(err),
            Self::NoPeer => None,
        }
    }
}
