use actix_server_alt::net::Stream;

/// A collection of regular used http protocols
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

    fn from_alpn<B: AsRef<[u8]>>(proto: B) -> Protocol {
        if proto.as_ref().windows(2).any(|window| window == b"h2") {
            Protocol::Http2
        } else {
            Protocol::Http1Tls
        }
    }
}

impl AsProtocol for Stream {
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
