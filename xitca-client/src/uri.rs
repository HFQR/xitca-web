use std::ops::Deref;

use crate::{error::InvalidUri, http::uri};

/// A new type of http::uri::Uri.
/// The purpose of it is to treat the Uri differently with Unix Socket Domain connection.
#[derive(Debug, Clone)]
pub enum Uri<'a> {
    Tcp(&'a uri::Uri),
    Tls(&'a uri::Uri),
    #[cfg(unix)]
    Unix(&'a uri::Uri),
}

impl Deref for Uri<'_> {
    type Target = uri::Uri;

    fn deref(&self) -> &Self::Target {
        match *self {
            Self::Tcp(uri) => uri,
            Self::Tls(uri) => uri,
            #[cfg(unix)]
            Self::Unix(uri) => uri,
        }
    }
}

impl<'a> Uri<'a> {
    pub(crate) fn try_parse(uri: &'a uri::Uri) -> Result<Self, InvalidUri> {
        match (uri.scheme_str(), uri.host(), uri.authority()) {
            (_, _, None) => Err(InvalidUri::MissingAuthority),
            (_, None, _) => Err(InvalidUri::MissingHost),
            (None, _, _) => Err(InvalidUri::MissingScheme),
            (Some("http" | "ws"), _, _) => Ok(Uri::Tcp(uri)),
            (Some("https" | "wss"), _, _) => Ok(Uri::Tls(uri)),
            #[cfg(unix)]
            (Some("unix"), _, _) => Ok(Uri::Unix(uri)),
            (Some(_), _, _) => Err(InvalidUri::UnknownScheme),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn uri_parse() {
        let uri = uri::Uri::from_static("http2://example.com");
        let err = Uri::try_parse(&uri).err().unwrap();
        assert!(matches!(err, InvalidUri::UnknownScheme));

        let uri = uri::Uri::from_static("/hello/world");
        let err = Uri::try_parse(&uri).err().unwrap();
        assert!(matches!(err, InvalidUri::MissingAuthority));

        let uri = uri::Uri::from_static("example.com");
        let err = Uri::try_parse(&uri).err().unwrap();
        assert!(matches!(err, InvalidUri::MissingScheme));

        let uri = uri::Uri::from_static("http://example.com");
        let _ = Uri::try_parse(&uri).unwrap();

        let uri = uri::Uri::from_static("unix://tmp/foo.socket");
        let uri = Uri::try_parse(&uri).unwrap();
        assert_eq!(uri.scheme_str().unwrap(), "unix");
        assert_eq!(uri.host().unwrap(), "tmp");
        assert_eq!(uri.path(), "/foo.socket");
    }
}
