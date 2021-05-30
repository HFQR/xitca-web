use std::{cmp, fmt::Write, io};

use bytes::{BufMut, BytesMut};
use http::{header::DATE, response::Parts, StatusCode, Version};
use log::{debug, warn};

use crate::body::{ResponseBody, ResponseBodySize};
use crate::util::date::DATE_VALUE_LENGTH;

use super::context::Context;
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

        let skip_len = match (status, version) {
            (StatusCode::SWITCHING_PROTOCOLS, _) => true,
            // TODO: add Request method to context and check in following branch.
            // Connect method with success status code should have no body.
            (s, _) if self.is_head_method() && s.is_success() => true,
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

        let mut date_header = false;

        for (name, value) in parts.headers.drain() {
            let name = name.expect("Handling optional header name is not implemented");

            match name {
                DATE => date_header = true,
                _ => {}
            }

            buf.put_slice(name.as_str().as_bytes());
            buf.put_slice(b": ");
            buf.put_slice(value.as_bytes());
            buf.put_slice(b"\r\n");
        }

        // set date header if there is not any.
        if !date_header {
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
    pub(super) fn encoder(&self) -> TransferEncoding {
        match *self {
            Self::None => TransferEncoding::empty(),
            Self::Bytes { ref bytes } => {
                if bytes.is_empty() {
                    TransferEncoding::eof()
                } else {
                    TransferEncoding::length(bytes.len() as u64)
                }
            }
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
    #[inline]
    pub fn empty() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(0),
        }
    }

    #[inline]
    pub fn eof() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Eof,
        }
    }

    #[inline]
    pub fn chunked() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Chunked(false),
        }
    }

    #[inline]
    pub fn length(len: u64) -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(len),
        }
    }

    /// Encode message. Return `EOF` state of encoder
    #[inline]
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
    #[inline]
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
