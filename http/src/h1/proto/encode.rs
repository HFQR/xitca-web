use tracing::{debug, warn};

use crate::{
    body::ResponseBodySize,
    bytes::BytesMut,
    date::DateTime,
    http::{
        header::{HeaderMap, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING},
        response::Parts,
        Extensions, StatusCode, Version,
    },
};

use super::{
    buf::WriteBuf,
    codec::TransferCoding,
    context::{ConnectionType, Context},
    error::{Parse, ProtoError},
};

impl<D, const MAX_HEADERS: usize> Context<'_, D, MAX_HEADERS>
where
    D: DateTime,
{
    pub(super) fn encode_continue<W>(&mut self, buf: &mut W)
    where
        W: WriteBuf,
    {
        buf.write_static(b"HTTP/1.1 100 Continue\r\n\r\n");
    }

    pub(super) fn encode_head<W>(
        &mut self,
        parts: Parts,
        size: ResponseBodySize,
        buf: &mut W,
    ) -> Result<TransferCoding, ProtoError>
    where
        W: WriteBuf,
    {
        buf.write_head(|buf| self.encode_head_inner(parts, size, buf))
    }

    fn encode_head_inner(
        &mut self,
        parts: Parts,
        size: ResponseBodySize,
        buf: &mut BytesMut,
    ) -> Result<TransferCoding, ProtoError> {
        let version = parts.version;
        let status = parts.status;

        // decide if content-length or transfer-encoding header would be skipped.
        let skip_len = match (status, version) {
            (StatusCode::SWITCHING_PROTOCOLS, _) => true,
            // Sending content-length or transfer-encoding header on 2xx response
            // to CONNECT is forbidden in RFC 7231.
            (s, _) if self.is_connect_method() && s.is_success() => true,
            (s, _) if s.is_informational() => {
                warn!(target: "h1_encode", "response with 1xx status code not supported");
                return Err(ProtoError::Parse(Parse::StatusCode));
            }
            _ => false,
        };

        // In some error cases, we don't know about the invalid message until already
        // pushing some bytes onto the `buf`. In those cases, we don't want to send
        // the half-pushed message, so rewind to before.
        // let orig_len = buf.len();

        // encode version, status code and reason
        encode_version_status_reason(buf, version, status);

        self.encode_headers(parts.headers, parts.extensions, size, buf, skip_len)
    }
}

#[inline]
fn encode_version_status_reason(buf: &mut BytesMut, version: Version, status: StatusCode) {
    // encode version, status code and reason
    match (version, status) {
        // happy path shortcut.
        (Version::HTTP_11, StatusCode::OK) => {
            buf.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
            return;
        }
        (Version::HTTP_11, _) => {
            buf.extend_from_slice(b"HTTP/1.1 ");
        }
        (Version::HTTP_10, _) => {
            buf.extend_from_slice(b"HTTP/1.0 ");
        }
        _ => {
            debug!(target: "h1_encode", "response with unexpected response version");
            buf.extend_from_slice(b"HTTP/1.1 ");
        }
    }

    // a reason MUST be written, as many parsers will expect it.
    let reason = status.canonical_reason().unwrap_or("<none>").as_bytes();
    let status = status.as_str().as_bytes();

    buf.reserve(status.len() + reason.len() + 3);
    buf.extend_from_slice(status);
    buf.extend_from_slice(b" ");
    buf.extend_from_slice(reason);
    buf.extend_from_slice(b"\r\n");
}

impl<D, const MAX_HEADERS: usize> Context<'_, D, MAX_HEADERS>
where
    D: DateTime,
{
    pub fn encode_headers(
        &mut self,
        mut headers: HeaderMap,
        mut extensions: Extensions,
        size: ResponseBodySize,
        buf: &mut BytesMut,
        mut skip_len: bool,
    ) -> Result<TransferCoding, ProtoError> {
        let mut skip_date = false;

        let mut encoding = TransferCoding::eof();

        for (name, value) in headers.drain() {
            let name = name.expect("Handling optional header name is not implemented");

            // TODO: more spec check needed. the current check barely does anything.
            match name {
                CONTENT_LENGTH => {
                    debug_assert!(!skip_len, "CONTENT_LENGTH header can not be set");
                    let value = value
                        .to_str()
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .ok_or(Parse::HeaderValue)?;
                    encoding = TransferCoding::length(value);
                    skip_len = true;
                }
                TRANSFER_ENCODING => {
                    debug_assert!(!skip_len, "TRANSFER_ENCODING header can not be set");
                    encoding = TransferCoding::encode_chunked();
                    skip_len = true;
                }
                CONNECTION => match self.ctype() {
                    // skip write header on close condition.
                    // the header is checked again and written properly afterwards.
                    ConnectionType::Close | ConnectionType::CloseForce => continue,
                    _ => {
                        for val in value.to_str().map_err(|_| Parse::HeaderValue)?.split(',') {
                            let val = val.trim();
                            if val.eq_ignore_ascii_case("close") {
                                self.set_ctype(ConnectionType::Close);
                            } else if val.eq_ignore_ascii_case("keep-alive") {
                                self.set_ctype(ConnectionType::KeepAlive);
                            } else if val.eq_ignore_ascii_case("upgrade") {
                                self.set_ctype(ConnectionType::Upgrade);
                                encoding = TransferCoding::upgrade();
                            }
                        }
                    }
                },
                DATE => skip_date = true,
                _ => {}
            }

            let name = name.as_str().as_bytes();
            let value = value.as_bytes();

            buf.reserve(name.len() + value.len() + 4);
            buf.extend_from_slice(name);
            buf.extend_from_slice(b": ");
            buf.extend_from_slice(value);
            buf.extend_from_slice(b"\r\n");
        }

        if matches!(self.ctype(), ConnectionType::Close | ConnectionType::CloseForce) {
            buf.extend_from_slice(b"connection: close\r\n");
        }

        // encode transfer-encoding or content-length
        if !skip_len {
            match size {
                ResponseBodySize::None => {
                    encoding = TransferCoding::eof();
                }
                ResponseBodySize::Stream => {
                    buf.extend_from_slice(b"transfer-encoding: chunked\r\n");
                    encoding = TransferCoding::encode_chunked();
                }
                ResponseBodySize::Sized(size) => {
                    let mut buffer = itoa::Buffer::new();
                    let buffer = buffer.format(size).as_bytes();

                    buf.reserve(buffer.len() + 18);
                    buf.extend_from_slice(b"content-length: ");
                    buf.extend_from_slice(buffer);
                    buf.extend_from_slice(b"\r\n");

                    encoding = TransferCoding::length(size as u64);
                }
            }
        }

        // set date header if there is not any.
        if !skip_date {
            buf.reserve(D::DATE_VALUE_LENGTH + 10);
            buf.extend_from_slice(b"date: ");
            self.date.with_date(|slice| buf.extend_from_slice(slice));
            buf.extend_from_slice(b"\r\n\r\n");
        } else {
            buf.extend_from_slice(b"\r\n");
        }

        // put header map back to cache.
        self.replace_headers(headers);

        // put extension back to cache;
        extensions.clear();
        self.replace_extensions(extensions);

        Ok(encoding)
    }
}
