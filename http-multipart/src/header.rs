use http::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_LENGTH, CONTENT_TYPE};
use httparse::{Error, EMPTY_HEADER};
use memchr::memmem;

use super::error::MultipartError;

/// Extract boundary info from headers.
pub(super) fn boundary<E>(headers: &HeaderMap) -> Result<&[u8], MultipartError<E>> {
    let header = headers
        .get(&CONTENT_TYPE)
        .ok_or(MultipartError::NoContentType)?
        .to_str()
        .map_err(|_| MultipartError::ParseContentType)?
        .as_bytes();

    let idx = memmem::find(header, b"boundary=").ok_or(MultipartError::Boundary)?;
    let start = idx + 9;
    let end = memmem::find(&header[start..], b";").unwrap_or(header.len());

    Ok(&header[start..end])
}

pub(super) fn parse_headers<E>(headers: &mut HeaderMap, slice: &[u8]) -> Result<(), MultipartError<E>> {
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

pub(super) fn check_headers<E>(headers: &HeaderMap) -> Result<(), MultipartError<E>> {
    let ct = headers
        .get(&CONTENT_TYPE)
        .and_then(|ct| ct.to_str().ok())
        .and_then(|ct| ct.parse().ok())
        .unwrap_or(mime::APPLICATION_OCTET_STREAM);

    // nested multipart stream is not supported
    if ct.type_() == mime::MULTIPART {
        return Err(MultipartError::Nested);
    }

    Ok(())
}

pub(super) fn content_length_opt<E>(headers: &HeaderMap) -> Result<Option<u64>, MultipartError<E>> {
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
