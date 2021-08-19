use std::cmp;

use bytes::{BufMut, Bytes, BytesMut};
use http::{
    header::{CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING},
    response::Parts,
    StatusCode, Version,
};
use tracing::{debug, warn};

use crate::body::ResponseBodySize;
use crate::util::date::DATE_VALUE_LENGTH;

use super::buf::WriteBuf;
use super::codec::TransferCoding;
use super::context::{ConnectionType, Context};
use super::error::{Parse, ProtoError};

impl<const MAX_HEADERS: usize> Context<'_, MAX_HEADERS> {
    pub(super) fn encode_continue<W, const WRITE_BUF_LIMIT: usize>(&mut self, buf: &mut W)
    where
        W: WriteBuf<WRITE_BUF_LIMIT>,
    {
        buf.write_static(b"HTTP/1.1 100 Continue\r\n\r\n");
    }

    pub(super) fn encode_head<W, const WRITE_BUF_LIMIT: usize>(
        &mut self,
        parts: Parts,
        size: ResponseBodySize,
        buf: &mut W,
    ) -> Result<TransferCoding, ProtoError>
    where
        W: WriteBuf<WRITE_BUF_LIMIT>,
    {
        buf.write_head(|buf| self.encode_head_inner(parts, size, buf))
    }

    fn encode_head_inner(
        &mut self,
        mut parts: Parts,
        size: ResponseBodySize,
        buf: &mut BytesMut,
    ) -> Result<TransferCoding, ProtoError> {
        let version = parts.version;
        let status = parts.status;

        // decide if content-length or transfer-encoding header would be skipped.
        let mut skip_len = match (status, version) {
            (StatusCode::SWITCHING_PROTOCOLS, _) => false,
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

        let mut skip_date = false;

        let mut encoding = TransferCoding::eof();

        for (name, value) in parts.headers.drain() {
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
                    encoding = TransferCoding::encode_chunked_from(self.ctype());
                    skip_len = true;
                }
                CONNECTION if self.is_force_close() => continue,
                CONNECTION => {
                    for val in value.to_str().map_err(|_| Parse::HeaderValue)?.split(',') {
                        let val = val.trim();

                        if val.eq_ignore_ascii_case("close") {
                            self.set_ctype(ConnectionType::Close);
                        } else if val.eq_ignore_ascii_case("keep-alive") {
                            self.set_ctype(ConnectionType::KeepAlive);
                        } else if val.eq_ignore_ascii_case("upgrade") {
                            self.set_ctype(ConnectionType::Upgrade);
                        }
                    }
                }
                DATE => skip_date = true,
                _ => {}
            }

            let name = name.as_str().as_bytes();
            let value = value.as_bytes();

            buf.reserve(name.len() + value.len() + 4);
            buf.put_slice(name);
            buf.put_slice(b": ");
            buf.put_slice(value);
            buf.put_slice(b"\r\n");
        }

        if self.is_force_close() {
            buf.put_slice(b"connection: close\r\n");
        }

        // encode transfer-encoding or content-length
        if !skip_len {
            match size {
                ResponseBodySize::None => {
                    encoding = TransferCoding::eof();
                }
                ResponseBodySize::Stream => {
                    buf.put_slice(b"transfer-encoding: chunked\r\n");
                    encoding = TransferCoding::encode_chunked_from(self.ctype());
                }
                ResponseBodySize::Sized(size) => {
                    let mut buffer = itoa::Buffer::new();
                    let buffer = buffer.format(size).as_bytes();

                    buf.reserve(buffer.len() + 18);
                    buf.put_slice(b"content-length: ");
                    buf.put_slice(buffer);
                    buf.put_slice(b"\r\n");

                    encoding = TransferCoding::length(size as u64);
                }
            }
        }

        // set date header if there is not any.
        if !skip_date {
            buf.reserve(DATE_VALUE_LENGTH + 10);
            buf.put_slice(b"date: ");
            buf.put_slice(self.date.borrow().date());
            buf.put_slice(b"\r\n\r\n");
        } else {
            buf.put_slice(b"\r\n");
        }

        // put header map back to cache.
        debug_assert!(parts.headers.is_empty());
        self.header = Some(parts.headers);

        // put extension back to cache;
        parts.extensions.clear();
        self.extensions = parts.extensions;

        Ok(encoding)
    }
}

#[inline]
fn encode_version_status_reason(buf: &mut BytesMut, version: Version, status: StatusCode) {
    // encode version, status code and reason
    match (version, status) {
        // happy path shortcut.
        (Version::HTTP_11, StatusCode::OK) => {
            buf.put_slice(b"HTTP/1.1 200 OK\r\n");
            return;
        }
        (Version::HTTP_11, _) => {
            buf.put_slice(b"HTTP/1.1 ");
        }
        (Version::HTTP_10, _) => {
            buf.put_slice(b"HTTP/1.0 ");
        }
        _ => {
            debug!(target: "h1_encode", "response with unexpected response version");
            buf.put_slice(b"HTTP/1.1 ");
        }
    }

    // a reason MUST be written, as many parsers will expect it.
    let reason = status.canonical_reason().unwrap_or("<none>").as_bytes();
    let status = status.as_str().as_bytes();

    buf.reserve(status.len() + reason.len() + 3);
    buf.put_slice(status);
    buf.put_slice(b" ");
    buf.put_slice(reason);
    buf.put_slice(b"\r\n");
}

impl TransferCoding {
    fn encode_chunked_from(ctype: ConnectionType) -> Self {
        if ctype == ConnectionType::Upgrade {
            Self::plain_chunked()
        } else {
            Self::encode_chunked()
        }
    }

    /// Encode message. Return `EOF` state of encoder
    pub(super) fn encode<W, const WRITE_BUF_LIMIT: usize>(&mut self, mut bytes: Bytes, buf: &mut W)
    where
        W: WriteBuf<WRITE_BUF_LIMIT>,
    {
        // Skip encode empty bytes.
        // This is to avoid unnecessary extending on h1::proto::buf::ListWriteBuf when user
        // provided empty bytes by accident.
        if bytes.is_empty() {
            return;
        }

        match *self {
            Self::PlainChunked => buf.write_buf(bytes),
            Self::EncodeChunked => buf.write_chunk(bytes),
            Self::Length(ref mut remaining) => {
                if *remaining > 0 {
                    let len = cmp::min(*remaining, bytes.len() as u64);
                    buf.write_buf(bytes.split_to(len as usize));
                    *remaining -= len as u64;
                }
            }
            Self::Eof => warn!(target: "h1_encode", "TransferCoding::Eof should not encode response body"),
            _ => unreachable!(),
        }
    }

    /// Encode eof. Return `EOF` state of encoder
    pub(super) fn encode_eof<W, const WRITE_BUF_LIMIT: usize>(&mut self, buf: &mut W)
    where
        W: WriteBuf<WRITE_BUF_LIMIT>,
    {
        match *self {
            Self::Eof | Self::PlainChunked | Self::Length(0) => {}
            Self::EncodeChunked => buf.write_static(b"0\r\n\r\n"),
            Self::Length(n) => unreachable!("UnexpectedEof for Length Body with {} remaining", n),
            _ => unreachable!(),
        }
    }
}
