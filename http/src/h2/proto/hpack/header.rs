use xitca_unsafe_collection::bytes::BytesStr;

use crate::{
    bytes::Bytes,
    http::{
        Method, StatusCode,
        header::{self, HeaderName, HeaderValue},
    },
};

use super::decoder::{DecoderError, NeedMore};

/// HTTP/2 Header
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Header<T = HeaderName> {
    Field { name: T, value: HeaderValue },
    // TODO: Change these types to `http::uri` types.
    Authority(BytesStr),
    Method(Method),
    Scheme(BytesStr),
    Path(BytesStr),
    Protocol(BytesStr),
    Status(StatusCode),
}

/// The header field name
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Name<'a> {
    Field(&'a HeaderName),
    Authority,
    Method,
    Scheme,
    Path,
    Protocol,
    Status,
}

pub fn len(name: &HeaderName, value: &HeaderValue) -> usize {
    let n: &str = name.as_ref();
    32 + n.len() + value.len()
}

impl Header<Option<HeaderName>> {
    pub fn reify(self) -> Result<Header, HeaderValue> {
        use self::Header::*;

        Ok(match self {
            Field { name: Some(n), value } => Field { name: n, value },
            Field { name: None, value } => return Err(value),
            Authority(v) => Authority(v),
            Method(v) => Method(v),
            Scheme(v) => Scheme(v),
            Path(v) => Path(v),
            Protocol(v) => Protocol(v),
            Status(v) => Status(v),
        })
    }
}

impl Header {
    pub fn new(name: Bytes, value: Bytes) -> Result<Header, DecoderError> {
        if name.is_empty() {
            return Err(DecoderError::NeedMore(NeedMore::UnexpectedEndOfStream));
        }
        if name[0] == b':' {
            match &name[1..] {
                b"authority" => {
                    let value = BytesStr::try_from(value)?;
                    Ok(Header::Authority(value))
                }
                b"method" => {
                    let method = Method::from_bytes(&value)?;
                    Ok(Header::Method(method))
                }
                b"scheme" => {
                    let value = BytesStr::try_from(value)?;
                    Ok(Header::Scheme(value))
                }
                b"path" => {
                    let value = BytesStr::try_from(value)?;
                    Ok(Header::Path(value))
                }
                b"protocol" => {
                    let value = BytesStr::try_from(value)?;
                    Ok(Header::Protocol(value))
                }
                b"status" => {
                    let status = StatusCode::from_bytes(&value)?;
                    Ok(Header::Status(status))
                }
                _ => Err(DecoderError::InvalidPseudoheader),
            }
        } else {
            // HTTP/2 requires lower case header names
            let name = HeaderName::from_lowercase(&name)?;
            let value = HeaderValue::from_bytes(&value)?;

            Ok(Header::Field { name, value })
        }
    }

    pub fn len(&self) -> usize {
        match *self {
            Header::Field { ref name, ref value } => len(name, value),
            Header::Authority(ref v) => 32 + 10 + v.len(),
            Header::Method(ref v) => 32 + 7 + v.as_ref().len(),
            Header::Scheme(ref v) => 32 + 7 + v.len(),
            Header::Path(ref v) => 32 + 5 + v.len(),
            Header::Protocol(ref v) => 32 + 9 + v.len(),
            Header::Status(_) => 32 + 7 + 3,
        }
    }

    /// Returns the header name
    pub fn name(&self) -> Name {
        match *self {
            Header::Field { ref name, .. } => Name::Field(name),
            Header::Authority(..) => Name::Authority,
            Header::Method(..) => Name::Method,
            Header::Scheme(..) => Name::Scheme,
            Header::Path(..) => Name::Path,
            Header::Protocol(..) => Name::Protocol,
            Header::Status(..) => Name::Status,
        }
    }

    pub fn value_slice(&self) -> &[u8] {
        match *self {
            Header::Field { ref value, .. } => value.as_ref(),
            Header::Authority(ref v) => v.as_bytes(),
            Header::Method(ref v) => v.as_ref().as_ref(),
            Header::Scheme(ref v) => v.as_bytes(),
            Header::Path(ref v) => v.as_bytes(),
            Header::Protocol(ref v) => v.as_bytes(),
            Header::Status(ref v) => v.as_str().as_ref(),
        }
    }

    pub fn value_eq(&self, other: &Header) -> bool {
        match *self {
            Header::Field { ref value, .. } => {
                let a = value;
                match *other {
                    Header::Field { ref value, .. } => a == value,
                    _ => false,
                }
            }
            Header::Authority(ref a) => match *other {
                Header::Authority(ref b) => a == b,
                _ => false,
            },
            Header::Method(ref a) => match *other {
                Header::Method(ref b) => a == b,
                _ => false,
            },
            Header::Scheme(ref a) => match *other {
                Header::Scheme(ref b) => a == b,
                _ => false,
            },
            Header::Path(ref a) => match *other {
                Header::Path(ref b) => a == b,
                _ => false,
            },
            Header::Protocol(ref a) => match *other {
                Header::Protocol(ref b) => a == b,
                _ => false,
            },
            Header::Status(ref a) => match *other {
                Header::Status(ref b) => a == b,
                _ => false,
            },
        }
    }

    pub fn is_sensitive(&self) -> bool {
        match *self {
            Header::Field { ref value, .. } => value.is_sensitive(),
            // TODO: Technically these other header values can be sensitive too.
            _ => false,
        }
    }

    pub fn skip_value_index(&self) -> bool {
        match *self {
            Header::Field { ref name, .. } => matches!(
                *name,
                header::AGE
                    | header::AUTHORIZATION
                    | header::CONTENT_LENGTH
                    | header::ETAG
                    | header::IF_MODIFIED_SINCE
                    | header::IF_NONE_MATCH
                    | header::LOCATION
                    | header::COOKIE
                    | header::SET_COOKIE
            ),
            Header::Path(..) => true,
            _ => false,
        }
    }
}

// Mostly for tests
impl From<Header> for Header<Option<HeaderName>> {
    fn from(src: Header) -> Self {
        match src {
            Header::Field { name, value } => Header::Field {
                name: Some(name),
                value,
            },
            Header::Authority(v) => Header::Authority(v),
            Header::Method(v) => Header::Method(v),
            Header::Scheme(v) => Header::Scheme(v),
            Header::Path(v) => Header::Path(v),
            Header::Protocol(v) => Header::Protocol(v),
            Header::Status(v) => Header::Status(v),
        }
    }
}

impl Name<'_> {
    pub fn into_entry(self, value: Bytes) -> Result<Header, DecoderError> {
        match self {
            Name::Field(name) => Ok(Header::Field {
                name: name.clone(),
                value: HeaderValue::from_bytes(&value)?,
            }),
            Name::Authority => Ok(Header::Authority(BytesStr::try_from(value)?)),
            Name::Method => Ok(Header::Method(Method::from_bytes(&value)?)),
            Name::Scheme => Ok(Header::Scheme(BytesStr::try_from(value)?)),
            Name::Path => Ok(Header::Path(BytesStr::try_from(value)?)),
            Name::Protocol => Ok(Header::Protocol(BytesStr::try_from(value)?)),
            Name::Status => {
                match StatusCode::from_bytes(&value) {
                    Ok(status) => Ok(Header::Status(status)),
                    // TODO: better error handling
                    Err(_) => Err(DecoderError::InvalidStatusCode),
                }
            }
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        match *self {
            Name::Field(ref name) => name.as_ref(),
            Name::Authority => b":authority",
            Name::Method => b":method",
            Name::Scheme => b":scheme",
            Name::Path => b":path",
            Name::Protocol => b":protocol",
            Name::Status => b":status",
        }
    }
}
