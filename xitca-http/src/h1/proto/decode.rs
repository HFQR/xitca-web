use std::{io, mem, task::Poll};

use bytes::{Buf, Bytes, BytesMut};
use http::{
    header::{HeaderMap, HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, EXPECT, TRANSFER_ENCODING, UPGRADE},
    Method, Request, Uri, Version,
};
use httparse::{Header, Status, EMPTY_HEADER};

use super::codec::{ChunkedState, Kind};
use super::context::{ConnectionType, Context};
use super::error::{Parse, ProtoError};

impl<const MAX_HEADERS: usize> Context<'_, MAX_HEADERS> {
    // decode head and generate request and body decoder.
    #[inline]
    pub(super) fn decode_head<const READ_BUF_LIMIT: usize>(
        &mut self,
        buf: &mut BytesMut,
    ) -> Result<Option<(Request<()>, TransferDecoding)>, ProtoError> {
        let mut headers = [EMPTY_HEADER; MAX_HEADERS];

        let mut req = httparse::Request::new(&mut headers);

        match req.parse(buf)? {
            Status::Complete(len) => {
                // Important: reset context state for new request.
                self.reset();

                let method = Method::from_bytes(req.method.unwrap().as_bytes())?;

                let uri = req.path.unwrap().parse::<Uri>()?;

                // Set connection type when doing version match.
                let version = if req.version.unwrap() == 1 {
                    // Default ctype is KeepAlive so set_ctype is skipped here.
                    Version::HTTP_11
                } else {
                    self.set_ctype(ConnectionType::Close);
                    Version::HTTP_10
                };

                // record the index of headers from the buffer.
                let mut header_idx = [HeaderIndex::new(); MAX_HEADERS];

                HeaderIndex::record(buf, req.headers, &mut header_idx);

                let headers_len = req.headers.len();

                // split the headers from buffer.
                let slice = buf.split_to(len).freeze();

                // pop a cached headermap or construct a new one.
                let mut headers = self.header.take().unwrap_or_else(HeaderMap::new);
                headers.reserve(headers_len);

                let mut decoder = TransferDecoding::eof();

                // write headers to headermap and update request states.
                for idx in &header_idx[..headers_len] {
                    let name = HeaderName::from_bytes(&slice[idx.name.0..idx.name.1]).unwrap();
                    let value = HeaderValue::from_maybe_shared(slice.slice(idx.value.0..idx.value.1)).unwrap();

                    match name {
                        TRANSFER_ENCODING => {
                            if version != Version::HTTP_11 {
                                return Err(ProtoError::Parse(Parse::HeaderName));
                            }

                            let chunked = value
                                .to_str()
                                .map_err(|_| ProtoError::Parse(Parse::HeaderName))?
                                .trim()
                                .eq_ignore_ascii_case("chunked");

                            if chunked {
                                decoder.reset(TransferDecoding::chunked())?;
                            }
                        }
                        CONTENT_LENGTH => {
                            let len = value
                                .to_str()
                                .map_err(|_| ProtoError::Parse(Parse::HeaderName))?
                                .parse::<u64>()
                                .map_err(|_| ProtoError::Parse(Parse::HeaderName))?;

                            if len != 0 {
                                decoder.reset(TransferDecoding::length(len))?;
                            }
                        }
                        CONNECTION => {
                            if let Ok(value) = value.to_str().map(|conn| conn.trim()) {
                                // Connection header would update context state.
                                if value.eq_ignore_ascii_case("keep-alive") {
                                    self.set_ctype(ConnectionType::KeepAlive);
                                } else if value.eq_ignore_ascii_case("close") {
                                    self.set_ctype(ConnectionType::Close);
                                } else if value.eq_ignore_ascii_case("upgrade") {
                                    // set decoder to upgrade variant.
                                    decoder = TransferDecoding::plain_chunked();
                                    self.set_ctype(ConnectionType::Upgrade);
                                }
                            }
                        }
                        EXPECT if value.as_bytes() == b"100-continue" => self.set_expect_header(),
                        // Upgrades are only allowed with HTTP/1.1
                        UPGRADE if version == Version::HTTP_11 => self.set_ctype(ConnectionType::Upgrade),
                        _ => {}
                    }

                    headers.append(name, value);
                }

                if method == Method::CONNECT {
                    self.set_ctype(ConnectionType::Upgrade);
                    // set method to context so it can pass method to response.
                    self.set_connect_method();
                    decoder = TransferDecoding::plain_chunked();
                }

                let mut req = Request::new(());

                let extensions = mem::take(&mut self.extensions);

                *req.method_mut() = method;
                *req.version_mut() = version;
                *req.uri_mut() = uri;
                *req.headers_mut() = headers;
                *req.extensions_mut() = extensions;

                Ok(Some((req, decoder)))
            }

            Status::Partial => {
                if buf.remaining() >= READ_BUF_LIMIT {
                    Err(ProtoError::Parse(Parse::HeaderTooLarge))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct HeaderIndex {
    name: (usize, usize),
    value: (usize, usize),
}

impl HeaderIndex {
    #[inline]
    fn new() -> Self {
        Self {
            name: (0, 0),
            value: (0, 0),
        }
    }

    #[inline]
    fn record(bytes: &[u8], headers: &[Header<'_>], indices: &mut [Self]) {
        let bytes_ptr = bytes.as_ptr() as usize;
        for (header, indices) in headers.iter().zip(indices.iter_mut()) {
            let name_start = header.name.as_ptr() as usize - bytes_ptr;
            let name_end = name_start + header.name.len();
            indices.name = (name_start, name_end);
            let value_start = header.value.as_ptr() as usize - bytes_ptr;
            let value_end = value_start + header.value.len();
            indices.value = (value_start, value_end);
        }
    }
}

/// Decoders to handle different Transfer-Encodings.
///
/// If a message body does not include a Transfer-Encoding, it *should*
/// include a Content-Length header.
#[derive(Debug, Clone, PartialEq)]
pub struct TransferDecoding {
    kind: Kind,
}

impl TransferDecoding {
    #[inline]
    pub const fn length(x: u64) -> TransferDecoding {
        TransferDecoding { kind: Kind::Length(x) }
    }

    #[inline]
    pub const fn chunked() -> TransferDecoding {
        TransferDecoding {
            kind: Kind::DecodeChunked(ChunkedState::Size, 0),
        }
    }

    #[inline]
    pub const fn plain_chunked() -> TransferDecoding {
        TransferDecoding {
            kind: Kind::PlainChunked,
        }
    }

    #[inline]
    pub const fn eof() -> TransferDecoding {
        TransferDecoding { kind: Kind::Eof }
    }

    #[inline]
    pub fn is_eof(&self) -> bool {
        matches!(self.kind, Kind::Eof)
    }

    pub fn reset(&mut self, other: Self) -> Result<(), ProtoError> {
        match (&self.kind, &other.kind) {
            (Kind::DecodeChunked(..), Kind::Length(..)) | (Kind::Length(..), Kind::DecodeChunked(..)) => {
                Err(ProtoError::Parse(Parse::HeaderName))
            }
            _ => {
                *self = other;
                Ok(())
            }
        }
    }
}

impl TransferDecoding {
    #[inline]
    pub(super) fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Bytes>> {
        match self.kind {
            Kind::Length(ref mut remaining) => {
                if *remaining == 0 {
                    Ok(Some(Bytes::new()))
                } else {
                    if src.is_empty() {
                        return Ok(None);
                    }
                    let len = src.len() as u64;
                    let buf = if *remaining > len {
                        *remaining -= len;
                        src.split().freeze()
                    } else {
                        let mut split = 0;
                        std::mem::swap(remaining, &mut split);
                        src.split_to(split as usize).freeze()
                    };
                    Ok(Some(buf))
                }
            }
            Kind::DecodeChunked(ref mut state, ref mut size) => {
                loop {
                    let mut buf = None;
                    // advances the chunked state
                    *state = match state.step(src, size, &mut buf) {
                        Poll::Pending => return Ok(None),
                        Poll::Ready(Ok(state)) => state,
                        Poll::Ready(Err(e)) => return Err(e),
                    };
                    if *state == ChunkedState::End {
                        return Ok(Some(Bytes::new()));
                    }
                    if let Some(buf) = buf {
                        return Ok(Some(buf));
                    }
                    if src.is_empty() {
                        return Ok(None);
                    }
                }
            }
            Kind::Eof | Kind::PlainChunked => {
                if src.is_empty() {
                    Ok(None)
                } else {
                    // TODO: hyper split 8kb here instead of take all.
                    Ok(Some(src.split().freeze()))
                }
            }
            _ => unreachable!(),
        }
    }
}
