use std::{
    convert::TryFrom,
    fmt,
    iter::{IntoIterator, Iterator},
    str::FromStr,
};

use http::{
    Extensions, HeaderMap, Method, StatusCode,
    header::{self, HeaderName, HeaderValue},
    uri::{self, Authority, Parts, PathAndQuery, Scheme, Uri},
};

use crate::http::Protocol;

use super::qpack::HeaderField;

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub struct Header {
    pseudo: Pseudo,
    fields: HeaderMap,
}

#[allow(clippy::len_without_is_empty)]
impl Header {
    /// Creates a new `Header` frame data suitable for sending a request
    pub fn request(method: Method, uri: Uri, fields: HeaderMap, ext: Extensions) -> Result<Self, HeaderError> {
        match (uri.authority(), fields.get("host")) {
            (None, None) => Err(HeaderError::MissingAuthority),
            (Some(a), Some(h)) if a.as_str() != h => Err(HeaderError::ContradictedAuthority),
            _ => Ok(Self {
                pseudo: Pseudo::request(method, uri, ext),
                fields,
            }),
        }
    }

    pub fn response(status: StatusCode, fields: HeaderMap) -> Self {
        Self {
            pseudo: Pseudo::response(status),
            fields,
        }
    }

    pub fn trailer(fields: HeaderMap) -> Self {
        Self {
            //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3
            //# Pseudo-header fields MUST NOT appear in trailer
            //# sections.
            pseudo: Pseudo::default(),
            fields,
        }
    }

    pub fn into_request_parts(self) -> Result<(Method, Uri, Option<Protocol>, HeaderMap), HeaderError> {
        let method = self.pseudo.method.clone().ok_or(HeaderError::MissingMethod)?;

        // RFC 9114 §4.4 + RFC 9220: classify the request shape so we can
        // validate the pseudo-header set.
        // - strict CONNECT  (CONNECT, no :protocol): MUST omit :scheme and :path.
        // - extended CONNECT (CONNECT, with :protocol) and regular requests:
        //   MUST include :scheme and :path.
        let is_strict_connect = method == Method::CONNECT && self.pseudo.protocol.is_none();
        match (is_strict_connect, self.pseudo.scheme.is_some()) {
            (true, true) => return Err(HeaderError::ConnectWithScheme),
            (false, false) => return Err(HeaderError::MissingScheme),
            _ => {}
        }
        match (is_strict_connect, self.pseudo.path.is_some()) {
            (true, true) => return Err(HeaderError::ConnectWithPath),
            (false, false) => return Err(HeaderError::MissingPath),
            _ => {}
        }

        let mut uri = Uri::builder();

        if let Some(path) = self.pseudo.path {
            uri = uri.path_and_query(path.as_str().as_bytes());
        }

        if let Some(scheme) = self.pseudo.scheme {
            uri = uri.scheme(scheme.as_str().as_bytes());
        }

        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //# If the :scheme pseudo-header field identifies a scheme that has a
        //# mandatory authority component (including "http" and "https"), the
        //# request MUST contain either an :authority pseudo-header field or a
        //# Host header field.

        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=TODO
        //# If the scheme does not have a mandatory authority component and none
        //# is provided in the request target, the request MUST NOT contain the
        //# :authority pseudo-header or Host header fields.
        match (self.pseudo.authority, self.fields.get("host")) {
            (None, None) => return Err(HeaderError::MissingAuthority),
            (Some(a), None) => uri = uri.authority(a.as_str().as_bytes()),
            (None, Some(h)) => uri = uri.authority(h.as_bytes()),
            //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
            //# If both fields are present, they MUST contain the same value.
            (Some(a), Some(h)) if a.as_str() != h => {
                return Err(HeaderError::ContradictedAuthority);
            }
            (Some(_), Some(h)) => uri = uri.authority(h.as_bytes()),
        }

        Ok((
            method,
            // When empty host field is built into an uri it fails
            //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
            //# If these fields are present, they MUST NOT be
            //# empty.
            uri.build().map_err(HeaderError::InvalidRequest)?,
            self.pseudo.protocol,
            self.fields,
        ))
    }

    /// Interpret a decoded HTTP/3 header block as trailers.
    //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3
    //# Pseudo-header fields MUST NOT appear in trailer
    //# sections.
    pub fn try_into_trailers(self) -> Result<HeaderMap, HeaderError> {
        if self.pseudo.len > 0 {
            return Err(HeaderError::PseudoInTrailer);
        }
        Ok(self.fields)
    }

    pub fn into_response_parts(self) -> Result<(StatusCode, HeaderMap), HeaderError> {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.2
        //= type=implication
        //# For responses, a single ":status" pseudo-header field is defined that
        //# carries the HTTP status code; see Section 15 of [HTTP].  This pseudo-
        //# header field MUST be included in all responses; otherwise, the
        //# response is malformed (see Section 4.1.2).
        Ok((self.pseudo.status.ok_or(HeaderError::MissingStatus)?, self.fields))
    }

    pub fn into_fields(self) -> HeaderMap {
        self.fields
    }

    pub fn len(&self) -> usize {
        self.pseudo.len() + self.fields.len()
    }

    pub fn size(&self) -> usize {
        self.pseudo.len() + self.fields.len()
    }

    #[cfg(test)]
    pub(crate) fn authory_mut(&mut self) -> &mut Option<Authority> {
        &mut self.pseudo.authority
    }
}

impl IntoIterator for Header {
    type Item = HeaderField;
    type IntoIter = HeaderIter;
    fn into_iter(self) -> Self::IntoIter {
        HeaderIter {
            pseudo: Some(self.pseudo),
            last_header_name: None,
            fields: self.fields.into_iter(),
        }
    }
}

pub struct HeaderIter {
    pseudo: Option<Pseudo>,
    last_header_name: Option<HeaderName>,
    fields: header::IntoIter<HeaderValue>,
}

impl Iterator for HeaderIter {
    type Item = HeaderField;

    fn next(&mut self) -> Option<Self::Item> {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3
        //# All pseudo-header fields MUST appear in the header section before
        //# regular header fields.
        if let Some(ref mut pseudo) = self.pseudo {
            if let Some(method) = pseudo.method.take() {
                return Some((":method", method.as_str()).into());
            }

            if let Some(scheme) = pseudo.scheme.take() {
                return Some((":scheme", scheme.as_str().as_bytes()).into());
            }

            if let Some(authority) = pseudo.authority.take() {
                return Some((":authority", authority.as_str().as_bytes()).into());
            }

            if let Some(path) = pseudo.path.take() {
                return Some((":path", path.as_str().as_bytes()).into());
            }

            if let Some(status) = pseudo.status.take() {
                return Some((":status", status.as_str()).into());
            }

            if let Some(protocol) = pseudo.protocol.take() {
                return Some((":protocol", protocol.as_str().as_bytes()).into());
            }
        }

        self.pseudo = None;

        for (new_header_name, header_value) in self.fields.by_ref() {
            if let Some(new) = new_header_name {
                self.last_header_name = Some(new);
            }
            if let (Some(n), v) = (&self.last_header_name, header_value) {
                return Some((n.as_str(), v.as_bytes()).into());
            }
        }

        None
    }
}

impl TryFrom<Vec<HeaderField>> for Header {
    type Error = HeaderError;
    fn try_from(headers: Vec<HeaderField>) -> Result<Self, Self::Error> {
        let mut fields = HeaderMap::with_capacity(headers.len());
        let mut pseudo = Pseudo::default();
        let mut seen_regular = false;

        for field in headers.into_iter() {
            let (name, value) = field.into_inner();
            let parsed = Field::parse(name, value)?;

            // Pseudo-header fields MUST appear before regular header fields
            // (RFC 9114 §4.3). Once we've seen a regular field, any further
            // pseudo is malformed.
            if !matches!(parsed, Field::Header(_)) && seen_regular {
                return Err(HeaderError::PseudoAfterRegular);
            }

            match parsed {
                Field::Method(m) => set_once(&mut pseudo.method, m, ":method", &mut pseudo.len)?,
                Field::Scheme(s) => set_once(&mut pseudo.scheme, s, ":scheme", &mut pseudo.len)?,
                Field::Authority(a) => set_once(&mut pseudo.authority, a, ":authority", &mut pseudo.len)?,
                Field::Path(p) => set_once(&mut pseudo.path, p, ":path", &mut pseudo.len)?,
                Field::Status(s) => set_once(&mut pseudo.status, s, ":status", &mut pseudo.len)?,
                Field::Protocol(p) => set_once(&mut pseudo.protocol, p, ":protocol", &mut pseudo.len)?,
                Field::Header((n, v)) => {
                    seen_regular = true;
                    fields.append(n, v);
                }
            }
        }

        Ok(Header { pseudo, fields })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, name: &'static str, len: &mut usize) -> Result<(), HeaderError> {
    if slot.is_some() {
        return Err(HeaderError::DuplicatePseudo(name));
    }
    *slot = Some(value);
    *len += 1;
    Ok(())
}

enum Field {
    Method(Method),
    Scheme(Scheme),
    Authority(Authority),
    Path(PathAndQuery),
    Status(StatusCode),
    Protocol(Protocol),
    Header((HeaderName, HeaderValue)),
}

impl Field {
    fn parse<N, V>(name: N, value: V) -> Result<Self, HeaderError>
    where
        N: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        let name = name.as_ref();
        if name.is_empty() {
            return Err(HeaderError::InvalidHeaderName("name is empty".into()));
        }

        //= https://www.rfc-editor.org/rfc/rfc9114#section-10.3
        //# Requests or responses containing invalid field names MUST be treated
        //# as malformed.

        //= https://www.rfc-editor.org/rfc/rfc9114#section-10.3
        //# Any request or response that contains a
        //# character not permitted in a field value MUST be treated as
        //# malformed.

        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.2
        //= type=implication
        //# A request or
        //# response containing uppercase characters in field names MUST be
        //# treated as malformed.

        if name[0] != b':' {
            //= https://www.rfc-editor.org/rfc/rfc9114#section-4.2.2
            //# An endpoint MUST NOT generate an HTTP/3 field section containing
            //# connection-specific fields; any message containing connection-
            //# specific fields MUST be treated as malformed.
            if is_connection_specific(name) {
                return Err(HeaderError::ConnectionSpecific(
                    String::from_utf8_lossy(name).into_owned(),
                ));
            }

            //= https://www.rfc-editor.org/rfc/rfc9114#section-4.2.2
            //# The only exception to this is the TE header field, which MAY
            //# be present in an HTTP/3 request; when it is, it MUST NOT
            //# contain any value other than "trailers".
            if name == b"te" && value.as_ref() != b"trailers" {
                return Err(HeaderError::InvalidTeValue);
            }

            return Ok(Field::Header((
                HeaderName::from_lowercase(name).map_err(|_| HeaderError::invalid_name(name))?,
                HeaderValue::from_bytes(value.as_ref()).map_err(|_| HeaderError::invalid_value(name, value))?,
            )));
        }

        Ok(match name {
            b":scheme" => Field::Scheme(try_value(name, value)?),
            //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
            //# If these fields are present, they MUST NOT be
            //# empty.
            b":authority" => Field::Authority(try_value(name, value)?),
            b":path" => Field::Path(try_value(name, value)?),
            b":method" => {
                Field::Method(Method::from_bytes(value.as_ref()).map_err(|_| HeaderError::invalid_value(name, value))?)
            }
            b":status" => Field::Status(
                StatusCode::from_bytes(value.as_ref()).map_err(|_| HeaderError::invalid_value(name, value))?,
            ),
            b":protocol" => Field::Protocol(try_value(name, value)?),
            _ => return Err(HeaderError::invalid_name(name)),
        })
    }
}

fn is_connection_specific(name: &[u8]) -> bool {
    matches!(
        name,
        b"connection" | b"keep-alive" | b"proxy-connection" | b"transfer-encoding" | b"upgrade"
    )
}

fn try_value<N, V, R>(name: N, value: V) -> Result<R, HeaderError>
where
    N: AsRef<[u8]>,
    V: AsRef<[u8]>,
    R: FromStr,
{
    let (name, value) = (name.as_ref(), value.as_ref());
    let s = std::str::from_utf8(value).map_err(|_| HeaderError::invalid_value(name, value))?;
    R::from_str(s).map_err(|_| HeaderError::invalid_value(name, value))
}

/// Pseudo-header fields have the same purpose as data from the first line of HTTP/1.X,
/// but are conveyed along with other headers. For example ':method' and ':path' in a
/// request, and ':status' in a response. They must be placed before all other fields,
/// start with ':', and be lowercase.
/// See RFC7540 section 8.1.2.1. for more details.
#[derive(Debug, Default)]
#[cfg_attr(test, derive(PartialEq, Clone))]
struct Pseudo {
    //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3
    //= type=implication
    //# Endpoints MUST NOT
    //# generate pseudo-header fields other than those defined in this
    //# document.

    // Request
    method: Option<Method>,
    scheme: Option<Scheme>,
    authority: Option<Authority>,
    path: Option<PathAndQuery>,

    // Response
    status: Option<StatusCode>,

    protocol: Option<Protocol>,

    len: usize,
}

#[allow(clippy::len_without_is_empty)]
impl Pseudo {
    fn request(method: Method, uri: Uri, ext: Extensions) -> Self {
        let Parts {
            scheme,
            authority,
            path_and_query,
            ..
        } = uri::Parts::from(uri);

        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=implication
        //# This pseudo-header field MUST NOT be empty for "http" or "https"
        //# URIs; "http" or "https" URIs that do not contain a path component
        //# MUST include a value of / (ASCII 0x2f).
        let path = path_and_query.map_or_else(
            || PathAndQuery::from_static("/"),
            |path| {
                if path.path().is_empty() && method != Method::OPTIONS {
                    PathAndQuery::from_static("/")
                } else {
                    path
                }
            },
        );

        // If the method is connect, the `:protocol` pseudo-header MAY be defined
        //
        // See: [https://www.rfc-editor.org/rfc/rfc8441#section-4]
        let protocol = if method == Method::CONNECT {
            ext.get::<Protocol>().copied()
        } else {
            None
        };

        // For standard CONNECT (that is, without :protocol pseudo-header) scheme and path
        // are not set. See: [https://www.rfc-editor.org/rfc/rfc9114#section-4.4]
        let (scheme, path) = if method == Method::CONNECT && protocol.is_none() {
            (None, None)
        } else {
            (scheme.or(Some(Scheme::HTTPS)), Some(path))
        };

        let len = 3 + authority.is_some() as usize + protocol.is_some() as usize;

        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3
        //= type=implication
        //# Pseudo-header fields defined for requests MUST NOT appear
        //# in responses; pseudo-header fields defined for responses MUST NOT
        //# appear in requests.

        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=implication
        //# All HTTP/3 requests MUST include exactly one value for the :method,
        //# :scheme, and :path pseudo-header fields, unless the request is a
        //# CONNECT request; see Section 4.4.
        Self {
            method: Some(method),
            scheme,
            authority,
            path,
            status: None,
            protocol,
            len,
        }
    }

    fn response(status: StatusCode) -> Self {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3
        //= type=implication
        //# Pseudo-header fields defined for requests MUST NOT appear
        //# in responses; pseudo-header fields defined for responses MUST NOT
        //# appear in requests.
        Pseudo {
            method: None,
            scheme: None,
            authority: None,
            path: None,
            status: Some(status),
            len: 1,
            protocol: None,
        }
    }

    fn len(&self) -> usize {
        self.len
    }
}

#[derive(Debug)]
pub enum HeaderError {
    InvalidHeaderName(String),
    InvalidHeaderValue(String),
    InvalidRequest(http::Error),
    MissingMethod,
    MissingStatus,
    MissingAuthority,
    ContradictedAuthority,
    ConnectionSpecific(String),
    InvalidTeValue,
    DuplicatePseudo(&'static str),
    PseudoAfterRegular,
    PseudoInTrailer,
    MissingScheme,
    MissingPath,
    ConnectWithScheme,
    ConnectWithPath,
}

impl HeaderError {
    fn invalid_name<N>(name: N) -> Self
    where
        N: AsRef<[u8]>,
    {
        HeaderError::InvalidHeaderName(format!("{:?}", name.as_ref()))
    }

    fn invalid_value<N, V>(name: N, value: V) -> Self
    where
        N: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        HeaderError::InvalidHeaderValue(format!(
            "{:?} {:?}",
            String::from_utf8_lossy(name.as_ref()),
            value.as_ref()
        ))
    }
}

impl std::error::Error for HeaderError {}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderError::InvalidHeaderName(h) => write!(f, "invalid header name: {}", h),
            HeaderError::InvalidHeaderValue(v) => write!(f, "invalid header value: {}", v),
            HeaderError::InvalidRequest(r) => write!(f, "invalid request: {}", r),
            HeaderError::MissingMethod => write!(f, "missing method in request headers"),
            HeaderError::MissingStatus => write!(f, "missing status in response headers"),
            HeaderError::MissingAuthority => write!(f, "missing authority"),
            HeaderError::ContradictedAuthority => {
                write!(f, "uri and authority field are in contradiction")
            }
            HeaderError::ConnectionSpecific(n) => {
                write!(f, "connection-specific header field {} is forbidden in HTTP/3", n)
            }
            HeaderError::InvalidTeValue => {
                write!(f, "te header field MUST only carry \"trailers\" in HTTP/3")
            }
            HeaderError::DuplicatePseudo(n) => write!(f, "pseudo-header {} appears more than once", n),
            HeaderError::PseudoAfterRegular => write!(f, "pseudo-header field appears after a regular field"),
            HeaderError::PseudoInTrailer => write!(f, "pseudo-header field appears in a trailer section"),
            HeaderError::MissingScheme => write!(f, "request is missing the :scheme pseudo-header"),
            HeaderError::MissingPath => write!(f, "request is missing the :path pseudo-header"),
            HeaderError::ConnectWithScheme => {
                write!(f, "CONNECT request without :protocol must not include :scheme")
            }
            HeaderError::ConnectWithPath => write!(f, "CONNECT request without :protocol must not include :path"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[test]
    fn request_has_no_authority_nor_host() {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=test
        //# If the :scheme pseudo-header field identifies a scheme that has a
        //# mandatory authority component (including "http" and "https"), the
        //# request MUST contain either an :authority pseudo-header field or a
        //# Host header field.
        let headers = Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":scheme", b"https").into(),
            (b":path", b"/").into(),
        ])
        .unwrap();
        assert!(headers.pseudo.authority.is_none());
        assert_matches!(headers.into_request_parts(), Err(HeaderError::MissingAuthority));
    }

    #[test]
    fn request_has_empty_authority() {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=test
        //# If these fields are present, they MUST NOT be
        //# empty.
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::GET.as_str()).into(),
                (b":authority", b"").into(),
            ]),
            Err(HeaderError::InvalidHeaderValue(_))
        );
    }

    #[test]
    fn request_has_empty_host() {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=test
        //# If these fields are present, they MUST NOT be
        //# empty.
        let headers = Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":scheme", b"https").into(),
            (b":path", b"/").into(),
            (b"host", b"").into(),
        ])
        .unwrap();
        assert_matches!(headers.into_request_parts(), Err(HeaderError::InvalidRequest(_)));
    }

    #[test]
    fn request_has_authority() {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=test
        //# If the :scheme pseudo-header field identifies a scheme that has a
        //# mandatory authority component (including "http" and "https"), the
        //# request MUST contain either an :authority pseudo-header field or a
        //# Host header field.
        let headers = Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":scheme", b"https").into(),
            (b":authority", b"test.com").into(),
            (b":path", b"/").into(),
        ])
        .unwrap();
        assert_matches!(headers.into_request_parts(), Ok(_));
    }

    #[test]
    fn request_has_host() {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=test
        //# If the :scheme pseudo-header field identifies a scheme that has a
        //# mandatory authority component (including "http" and "https"), the
        //# request MUST contain either an :authority pseudo-header field or a
        //# Host header field.
        let headers = Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":scheme", b"https").into(),
            (b":path", b"/").into(),
            (b"host", b"test.com").into(),
        ])
        .unwrap();
        assert!(headers.pseudo.authority.is_none());
        assert_matches!(headers.into_request_parts(), Ok(_));
    }

    #[test]
    fn request_has_same_host_and_authority() {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=test
        //# If both fields are present, they MUST contain the same value.
        let headers = Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":scheme", b"https").into(),
            (b":authority", b"test.com").into(),
            (b":path", b"/").into(),
            (b"host", b"test.com").into(),
        ])
        .unwrap();
        assert_matches!(headers.into_request_parts(), Ok(_));
    }
    #[test]
    fn request_has_different_host_and_authority() {
        //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
        //= type=test
        //# If both fields are present, they MUST contain the same value.
        let headers = Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":scheme", b"https").into(),
            (b":authority", b"authority.com").into(),
            (b":path", b"/").into(),
            (b"host", b"host.com").into(),
        ])
        .unwrap();
        assert_matches!(headers.into_request_parts(), Err(HeaderError::ContradictedAuthority));
    }

    #[test]
    fn rejects_connection_specific_header() {
        for name in [
            b"connection".as_slice(),
            b"keep-alive",
            b"proxy-connection",
            b"transfer-encoding",
            b"upgrade",
        ] {
            assert_matches!(
                Header::try_from(vec![
                    (b":method", Method::GET.as_str()).into(),
                    (b":authority", b"test.com").into(),
                    (name, b"close").into(),
                ]),
                Err(HeaderError::ConnectionSpecific(_))
            );
        }
    }

    #[test]
    fn accepts_te_trailers_only() {
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::GET.as_str()).into(),
                (b":authority", b"test.com").into(),
                (b"te", b"trailers").into(),
            ]),
            Ok(_)
        );
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::GET.as_str()).into(),
                (b":authority", b"test.com").into(),
                (b"te", b"gzip").into(),
            ]),
            Err(HeaderError::InvalidTeValue)
        );
    }

    #[test]
    fn rejects_duplicate_pseudo() {
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::GET.as_str()).into(),
                (b":method", Method::POST.as_str()).into(),
                (b":authority", b"test.com").into(),
            ]),
            Err(HeaderError::DuplicatePseudo(":method"))
        );
    }

    #[test]
    fn regular_request_requires_scheme_and_path() {
        // Missing :scheme.
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::GET.as_str()).into(),
                (b":authority", b"test.com").into(),
                (b":path", b"/").into(),
            ])
            .unwrap()
            .into_request_parts(),
            Err(HeaderError::MissingScheme)
        );
        // Missing :path.
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::GET.as_str()).into(),
                (b":scheme", b"https").into(),
                (b":authority", b"test.com").into(),
            ])
            .unwrap()
            .into_request_parts(),
            Err(HeaderError::MissingPath)
        );
    }

    #[test]
    fn strict_connect_rejects_scheme_or_path() {
        // CONNECT with :scheme.
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::CONNECT.as_str()).into(),
                (b":scheme", b"https").into(),
                (b":authority", b"test.com:443").into(),
            ])
            .unwrap()
            .into_request_parts(),
            Err(HeaderError::ConnectWithScheme)
        );
        // CONNECT with :path.
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::CONNECT.as_str()).into(),
                (b":authority", b"test.com:443").into(),
                (b":path", b"/").into(),
            ])
            .unwrap()
            .into_request_parts(),
            Err(HeaderError::ConnectWithPath)
        );
    }

    #[test]
    fn strict_connect_accepts_authority_only() {
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::CONNECT.as_str()).into(),
                (b":authority", b"test.com:443").into(),
            ])
            .unwrap()
            .into_request_parts(),
            Ok(_)
        );
    }

    #[test]
    fn rejects_pseudo_after_regular() {
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::GET.as_str()).into(),
                (b"x-foo", b"bar").into(),
                (b":authority", b"test.com").into(),
            ]),
            Err(HeaderError::PseudoAfterRegular)
        );
    }

    #[test]
    fn preserves_duplicate_headers() {
        let headers = Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":authority", b"test.com").into(),
            (b"set-cookie", b"foo=foo").into(),
            (b"set-cookie", b"bar=bar").into(),
            (b"other-header", b"other-header-value").into(),
        ])
        .unwrap();

        assert_eq!(
            headers
                .clone()
                .into_iter()
                .filter(|h| h.name.as_ref() == b"set-cookie")
                .collect::<Vec<_>>(),
            vec![
                HeaderField {
                    name: std::borrow::Cow::Borrowed(b"set-cookie"),
                    value: std::borrow::Cow::Borrowed(b"foo=foo")
                },
                HeaderField {
                    name: std::borrow::Cow::Borrowed(b"set-cookie"),
                    value: std::borrow::Cow::Borrowed(b"bar=bar")
                }
            ]
        );
        assert_eq!(
            headers
                .into_iter()
                .filter(|h| h.name.as_ref() == b"other-header")
                .collect::<Vec<_>>(),
            vec![HeaderField {
                name: std::borrow::Cow::Borrowed(b"other-header"),
                value: std::borrow::Cow::Borrowed(b"other-header-value")
            },]
        );
    }
}
