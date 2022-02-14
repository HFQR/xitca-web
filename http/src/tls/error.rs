use std::fmt;

use crate::error::HttpServiceError;

pub enum TlsError {
    Infallible,
    #[cfg(feature = "openssl")]
    Openssl(super::openssl::OpensslError),
    #[cfg(feature = "rustls")]
    Rustls(super::rustls::RustlsError),
    #[cfg(feature = "native-tls")]
    NativeTls(super::native_tls::NativeTlsError),
}

impl fmt::Debug for TlsError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Infallible => unreachable!("Infallible error should never happen"),
            #[cfg(feature = "openssl")]
            Self::Openssl(ref e) => write!(_f, "{:?}", e),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref e) => write!(_f, "{:?}", e),
            #[cfg(feature = "native-tls")]
            Self::NativeTls(ref e) => write!(_f, "{:?}", e),
        }
    }
}

impl<S, B> From<TlsError> for HttpServiceError<S, B> {
    fn from(e: TlsError) -> Self {
        Self::Tls(e)
    }
}

#[cfg(feature = "openssl")]
impl<S, B> From<super::openssl::OpensslError> for HttpServiceError<S, B> {
    fn from(e: super::openssl::OpensslError) -> Self {
        Self::Tls(e.into())
    }
}

#[cfg(feature = "rustls")]
impl<S, B> From<super::rustls::RustlsError> for HttpServiceError<S, B> {
    fn from(e: super::rustls::RustlsError) -> Self {
        Self::Tls(e.into())
    }
}

#[cfg(feature = "native-tls")]
impl<S, B> From<super::native_tls::NativeTlsError> for HttpServiceError<S, B> {
    fn from(e: super::native_tls::NativeTlsError) -> Self {
        Self::Tls(e.into())
    }
}
