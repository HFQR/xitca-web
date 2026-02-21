use std::{error, fmt};

use crate::error::HttpServiceError;

pub enum TlsError {
    Infallible,
    #[cfg(feature = "openssl")]
    Openssl(super::openssl::OpensslError),
    #[cfg(feature = "rustls")]
    Rustls(super::rustls::RustlsError),
    #[cfg(feature = "native-tls")]
    NativeTls(super::native_tls::NativeTlsError),
    OtherTls(Box<dyn error::Error + Send + Sync>),
}

impl fmt::Debug for TlsError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Infallible => unreachable!("Infallible error should never happen"),
            #[cfg(feature = "openssl")]
            Self::Openssl(ref e) => fmt::Debug::fmt(e, _f),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref e) => fmt::Debug::fmt(e, _f),
            #[cfg(feature = "native-tls")]
            Self::NativeTls(ref e) => fmt::Debug::fmt(e, _f),
            Self::OtherTls(ref e) => fmt::Debug::fmt(e, _f),
        }
    }
}

impl fmt::Display for TlsError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Infallible => unreachable!("Infallible error should never happen"),
            #[cfg(feature = "openssl")]
            Self::Openssl(ref e) => fmt::Debug::fmt(e, _f),
            #[cfg(feature = "rustls")]
            Self::Rustls(ref e) => fmt::Debug::fmt(e, _f),
            #[cfg(feature = "native-tls")]
            Self::NativeTls(ref e) => fmt::Display::fmt(e, _f),
            Self::OtherTls(ref e) => fmt::Display::fmt(e, _f),
        }
    }
}

impl error::Error for TlsError {}

impl From<Box<dyn error::Error + Send + Sync>> for TlsError {
    fn from(e: Box<dyn error::Error + Send + Sync>) -> Self {
        Self::OtherTls(e)
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
