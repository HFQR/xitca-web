use xitca_io::net::TcpStream;

use crate::http::Version;

/// A helper trait for get a protocol from certain types.
pub trait AsVersion {
    fn as_version(&self) -> Version;

    fn from_alpn<B: AsRef<[u8]>>(proto: B) -> Version {
        if proto.as_ref().windows(2).any(|window| window == b"h2") {
            Version::HTTP_2
        } else {
            Version::HTTP_11
        }
    }
}

impl AsVersion for xitca_io::net::Stream {
    #[inline]
    fn as_version(&self) -> Version {
        match *self {
            Self::Tcp(ref tcp) => tcp.as_version(),
            #[cfg(unix)]
            Self::Unix(..) => Version::HTTP_11,
            #[cfg(feature = "http3")]
            Self::Udp(..) => Version::HTTP_3,
        }
    }
}

impl AsVersion for TcpStream {
    #[inline]
    fn as_version(&self) -> Version {
        Version::HTTP_11
    }
}
