/// A collection of regular used protocols
#[derive(Copy, Clone, PartialOrd, PartialEq, Debug)]
pub enum Protocol {
    /// Plain Http1
    Http1,
    /// Http1 over Tls
    Http1Tls,
    Http2,
    Http3,
}

/// A helper trait for get a protocol from certain types.
pub trait AsProtocol {
    fn as_protocol(&self) -> Protocol;
}

impl AsProtocol for super::Stream {
    #[inline]
    fn as_protocol(&self) -> Protocol {
        match *self {
            Self::Tcp(..) => Protocol::Http1,
            #[cfg(unix)]
            Self::Unix(..) => Protocol::Http1,
            #[cfg(feature = "http3")]
            Self::Udp(..) => Protocol::Http3,
        }
    }
}
