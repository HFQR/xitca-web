use std::{cmp, fmt::Write, io};

use bytes::{BufMut, BytesMut};
use http::{
    header::{CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING},
    response::Parts,
    Method, StatusCode, Version,
};
use log::{debug, warn};

use crate::body::{ResponseBody, ResponseBodySize};
use crate::util::date::DATE_VALUE_LENGTH;

use super::context::{ConnectionType, Context};
use super::error::{Parse, ProtoError};

impl Context<'_> {
    pub(super) fn encode_head(
        &mut self,
        mut parts: Parts,
        size: ResponseBodySize,
        buf: &mut BytesMut,
    ) -> Result<(), ProtoError> {
        let version = parts.version;
        let status = parts.status;

        // decide if content-length or transfer-encoding header would be skipped.
        let mut skip_len = match (status, version) {
            (StatusCode::SWITCHING_PROTOCOLS, _) => true,
            // Sending content-length or transfer-encoding header on 2xx response
            // to CONNECT is forbidden in RFC 7231.
            (s, _) if self.req_method() == Method::CONNECT && s.is_success() => true,
            (s, _) if s.is_informational() => {
                warn!("response with 1xx status code not supported");
                return Err(ProtoError::Parse(Parse::StatusCode));
            }
            _ => false,
        };

        // In some error cases, we don't know about the invalid message until already
        // pushing some bytes onto the `buf`. In those cases, we don't want to send
        // the half-pushed message, so rewind to before.
        let orig_len = buf.len();

        // encode version, status code and reason
        encode_version_status_reason(buf, version, status);

        let mut skip_date = false;

        for (name, value) in parts.headers.drain() {
            let name = name.expect("Handling optional header name is not implemented");

            // TODO: more spec check needed. the current check barely does anything.
            match name {
                CONTENT_LENGTH => {
                    debug_assert!(!skip_len, "CONTENT_LENGTH header can not be set");
                    skip_len = true;
                }
                TRANSFER_ENCODING => {
                    debug_assert!(!skip_len, "TRANSFER_ENCODING header can not be set");
                    skip_len = true;
                }
                CONNECTION => {
                    for val in value.to_str().map_err(|_| Parse::HeaderValue)?.split(',') {
                        let val = val.trim();

                        if val.eq_ignore_ascii_case("close") {
                            self.ctype = ConnectionType::Close;
                            break;
                        } else if val.eq_ignore_ascii_case("keep-alive") {
                            self.ctype = ConnectionType::KeepAlive;
                            break;
                        }
                    }
                }
                DATE => skip_date = true,
                _ => {}
            }

            buf.put_slice(name.as_str().as_bytes());
            buf.put_slice(b": ");
            buf.put_slice(value.as_bytes());
            buf.put_slice(b"\r\n");
        }

        // encode transfer-encoding or content-length
        if !skip_len {
            match size {
                ResponseBodySize::None => {}
                ResponseBodySize::Stream => buf.put_slice(b"transfer-encoding: chunked\r\n"),
                ResponseBodySize::Sized(size) => {
                    let mut buffer = itoa::Buffer::new();
                    buf.put_slice(b"content-length: ");
                    buf.put_slice(buffer.format(size).as_bytes());
                    buf.put_slice(b"\r\n");
                }
            }
        }

        // set date header if there is not any.
        if !skip_date {
            buf.reserve(DATE_VALUE_LENGTH + 8);
            buf.put_slice(b"date: ");
            buf.put_slice(self.date.get().as_bytes());
            buf.put_slice(b"\r\n\r\n");
        } else {
            buf.put_slice(b"\r\n");
        }

        // put header map back to cache.
        self.header_cache = Some(parts.headers);

        Ok(())
    }
}

fn encode_version_status_reason<B: BufMut>(buf: &mut B, version: Version, status: StatusCode) {
    // encode version, status code and reason
    match (version, status) {
        // happy path shortcut.
        (Version::HTTP_11, StatusCode::OK) => {
            buf.put_slice(b"HTTP/1.1 200 OK\r\n");
            return;
        }
        (Version::HTTP_10, _) => {
            buf.put_slice(b"HTTP/1.0 ");
        }
        (Version::HTTP_11, _) => {
            buf.put_slice(b"HTTP/1.1 ");
        }
        _ => {
            debug!("response with unexpected response version");
            buf.put_slice(b"HTTP/1.1 ");
        }
    }

    buf.put_slice(status.as_str().as_bytes());
    buf.put_slice(b" ");
    // a reason MUST be written, as many parsers will expect it.
    buf.put_slice(status.canonical_reason().unwrap_or("<none>").as_bytes());
    buf.put_slice(b"\r\n");
}

impl<B> ResponseBody<B> {
    /// `TransferEncoding` must match the behavior of `Stream` impl of `ResponseBody`.
    /// Which means when `Stream::poll_next` returns Some(`Stream::Item`) the encoding
    /// must be able to encode data. And when it returns `None` it must valid to encode
    /// eof which would finish the encoding.
    pub(super) fn encoder(&self) -> TransferEncoding {
        match *self {
            // None body would return None on first poll of ResponseBody as Stream.
            // an eof encoding would return Ok(()) afterward.
            Self::None => TransferEncoding::eof(),
            // Empty bytes would return None on first poll of ResponseBody as Stream.
            // A length encoding would see the remainning length is 0 and return Ok(()).
            Self::Bytes { ref bytes } => TransferEncoding::length(bytes.len() as u64),
            Self::Stream { .. } => TransferEncoding::chunked(),
        }
    }
}

/// Encoders to handle different Transfer-Encodings.
#[derive(Debug)]
pub(crate) struct TransferEncoding {
    kind: TransferEncodingKind,
}

#[derive(Debug, PartialEq, Clone)]
enum TransferEncodingKind {
    /// An Encoder for when Transfer-Encoding includes `chunked`.
    Chunked(bool),
    /// An Encoder for when Content-Length is set.
    ///
    /// Enforces that the body is not longer than the Content-Length header.
    Length(u64),
    /// An Encoder for when Content-Length is not known.
    ///
    /// Application decides when to stop writing.
    Eof,
}

impl TransferEncoding {
    #[inline(always)]
    pub fn eof() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Eof,
        }
    }

    #[inline(always)]
    pub fn chunked() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Chunked(false),
        }
    }

    #[inline(always)]
    pub fn length(len: u64) -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(len),
        }
    }

    /// Encode message. Return `EOF` state of encoder
    #[inline(always)]
    pub fn encode(&mut self, msg: &[u8], buf: &mut BytesMut) -> io::Result<bool> {
        match self.kind {
            TransferEncodingKind::Eof => {
                let eof = msg.is_empty();
                buf.extend_from_slice(msg);
                Ok(eof)
            }
            TransferEncodingKind::Chunked(ref mut eof) => {
                if *eof {
                    return Ok(true);
                }

                if msg.is_empty() {
                    *eof = true;
                    buf.extend_from_slice(b"0\r\n\r\n");
                } else {
                    writeln!(buf, "{:X}\r", msg.len()).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

                    buf.reserve(msg.len() + 2);
                    buf.extend_from_slice(msg);
                    buf.extend_from_slice(b"\r\n");
                }
                Ok(*eof)
            }
            TransferEncodingKind::Length(ref mut remaining) => {
                if *remaining > 0 {
                    if msg.is_empty() {
                        return Ok(*remaining == 0);
                    }
                    let len = cmp::min(*remaining, msg.len() as u64);

                    buf.extend_from_slice(&msg[..len as usize]);

                    *remaining -= len as u64;
                    Ok(*remaining == 0)
                } else {
                    Ok(true)
                }
            }
        }
    }

    /// Encode eof. Return `EOF` state of encoder
    #[inline(always)]
    pub fn encode_eof(&mut self, buf: &mut BytesMut) -> io::Result<()> {
        match self.kind {
            TransferEncodingKind::Eof => Ok(()),
            TransferEncodingKind::Length(rem) => {
                if rem != 0 {
                    Err(io::Error::new(io::ErrorKind::UnexpectedEof, ""))
                } else {
                    Ok(())
                }
            }
            TransferEncodingKind::Chunked(ref mut eof) => {
                if !*eof {
                    *eof = true;
                    buf.extend_from_slice(b"0\r\n\r\n");
                }
                Ok(())
            }
        }
    }
}
