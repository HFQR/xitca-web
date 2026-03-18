use http::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_LENGTH, CONTENT_TYPE};
use httparse::{Error, EMPTY_HEADER};
use memchr::memmem;

use super::error::MultipartError;

/// Extract boundary info from headers.
pub(super) fn boundary(headers: &HeaderMap) -> Result<&[u8], MultipartError> {
    let header = headers
        .get(&CONTENT_TYPE)
        .ok_or(MultipartError::NoContentType)?
        .to_str()
        .map_err(|_| MultipartError::ParseContentType)?
        .as_bytes();

    let idx = memmem::find(header, b"boundary=").ok_or(MultipartError::Boundary)?;
    let start = idx + 9;
    let end = memmem::find(&header[start..], b";")
        .map(|i| start + i)
        .unwrap_or(header.len());

    // RFC 2045 §5.1: the boundary parameter may be a quoted-string
    // (e.g. boundary="ab cd") or an unquoted token. Either form may have
    // surrounding OWS from header folding, so trim first, then strip quotes.
    let mut boundary = header[start..end].trim_ascii();

    if let Some(quoted) = boundary.strip_prefix(b"\"") {
        let end = memchr::memchr(b'"', quoted).ok_or(MultipartError::Boundary)?;
        boundary = &quoted[..end];
    }

    if boundary.is_empty() {
        return Err(MultipartError::Boundary);
    }

    Ok(boundary)
}

pub(super) fn parse_headers(headers: &mut HeaderMap, slice: &[u8]) -> Result<(), MultipartError> {
    let mut hdrs = [EMPTY_HEADER; 16];
    match httparse::parse_headers(slice, &mut hdrs)? {
        httparse::Status::Complete((_, hdrs)) => {
            debug_assert!(headers.is_empty());
            headers.reserve(hdrs.len());

            for h in hdrs {
                let name = HeaderName::try_from(h.name).unwrap();
                let value = HeaderValue::try_from(h.value).unwrap();
                headers.append(name, value);
            }

            Ok(())
        }
        httparse::Status::Partial => Err(Error::TooManyHeaders.into()),
    }
}

pub(super) fn check_headers(headers: &HeaderMap) -> Result<(), MultipartError> {
    let nested = headers
        .get(&CONTENT_TYPE)
        .and_then(|ct| ct.to_str().ok())
        .filter(|ct| ct.trim_start().starts_with("multipart/"))
        .is_some();

    // nested multipart stream is not supported
    if nested {
        Err(MultipartError::Nested)
    } else {
        Ok(())
    }
}

pub(super) fn content_length_opt(headers: &HeaderMap) -> Result<Option<u64>, MultipartError> {
    match headers.get(&CONTENT_LENGTH) {
        Some(len) => {
            let len = len
                .to_str()
                .map_err(|_| Error::HeaderValue)?
                .parse()
                .map_err(|_| Error::HeaderValue)?;
            Ok(Some(len))
        }
        None => Ok(None),
    }
}
