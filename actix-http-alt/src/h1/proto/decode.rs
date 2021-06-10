use std::{io, task::Poll};

use bytes::{Buf, Bytes, BytesMut};
use http::{
    header::{HeaderMap, HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, EXPECT, TRANSFER_ENCODING, UPGRADE},
    Method, Request, Uri, Version,
};
use httparse::{Header, Status, EMPTY_HEADER};

use super::context::{ConnectionType, Context};
use super::error::{Parse, ProtoError};

/// No particular reason. Copied from `actix-http` crate.
const MAX_HEADERS: usize = 96;

impl<const HEAD_LIMIT: usize> Context<'_, HEAD_LIMIT> {
    // decode head and generate request and body decoder.
    pub(super) fn decode_head(
        &mut self,
        buf: &mut BytesMut,
    ) -> Result<Option<(Request<()>, RequestBodyDecoder)>, ProtoError> {
        let mut headers = [EMPTY_HEADER; MAX_HEADERS];

        let mut req = httparse::Request::new(&mut headers);

        match req.parse(buf)? {
            Status::Complete(len) => {
                // Important: reset context state for new request.
                self.reset();

                let method = Method::from_bytes(req.method.unwrap().as_bytes())?;

                // set method to context so it can pass method to response.
                self.set_method(method.clone());

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
                let mut headers = self.header_cache.take().unwrap_or_else(HeaderMap::new);
                headers.reserve(headers_len);

                let mut decoder = RequestBodyDecoder::eof();

                // write headers to headermap and update request states.
                for idx in &header_idx[..headers_len] {
                    let name = HeaderName::from_bytes(&slice[idx.name.0..idx.name.1]).unwrap();
                    let value = HeaderValue::from_maybe_shared(slice.slice(idx.value.0..idx.value.1)).unwrap();

                    match name {
                        TRANSFER_ENCODING => {
                            if version != Version::HTTP_11 {
                                return Err(ProtoError::Parse(Parse::Header));
                            }

                            let chunked = value
                                .to_str()
                                .map_err(|_| ProtoError::Parse(Parse::Header))?
                                .trim()
                                .eq_ignore_ascii_case("chunked");

                            if chunked {
                                decoder.reset(RequestBodyDecoder::chunked())?;
                            }
                        }
                        CONTENT_LENGTH => {
                            let len = value
                                .to_str()
                                .map_err(|_| ProtoError::Parse(Parse::Header))?
                                .parse::<u64>()
                                .map_err(|_| ProtoError::Parse(Parse::Header))?;

                            if len != 0 {
                                decoder.reset(RequestBodyDecoder::length(len))?;
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
                                    decoder = RequestBodyDecoder::plain_chunked();
                                    self.set_ctype(ConnectionType::Upgrade);
                                }
                            }
                        }
                        EXPECT if value.as_bytes() == b"100-continue" => self.set_expect(),
                        // Upgrades are only allowed with HTTP/1.1
                        UPGRADE if version == Version::HTTP_11 => self.set_ctype(ConnectionType::Upgrade),
                        _ => {}
                    }

                    headers.append(name, value);
                }

                if method == Method::CONNECT {
                    self.set_ctype(ConnectionType::Upgrade);
                    decoder = RequestBodyDecoder::plain_chunked();
                }

                let mut req = Request::new(());

                *req.method_mut() = method;
                *req.version_mut() = version;
                *req.uri_mut() = uri;
                *req.headers_mut() = headers;

                Ok(Some((req, decoder)))
            }

            Status::Partial => {
                if buf.remaining() > HEAD_LIMIT {
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
    fn new() -> Self {
        Self {
            name: (0, 0),
            value: (0, 0),
        }
    }

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
pub struct RequestBodyDecoder {
    kind: Kind,
}

impl RequestBodyDecoder {
    #[inline(always)]
    pub fn length(x: u64) -> RequestBodyDecoder {
        RequestBodyDecoder { kind: Kind::Length(x) }
    }

    #[inline(always)]
    pub fn chunked() -> RequestBodyDecoder {
        RequestBodyDecoder {
            kind: Kind::Chunked(ChunkedState::Size, 0),
        }
    }

    #[inline(always)]
    pub fn plain_chunked() -> RequestBodyDecoder {
        RequestBodyDecoder {
            kind: Kind::PlainChunked,
        }
    }

    #[inline(always)]
    pub fn eof() -> RequestBodyDecoder {
        RequestBodyDecoder { kind: Kind::Eof }
    }

    #[inline(always)]
    pub fn is_eof(&self) -> bool {
        matches!(self.kind, Kind::Eof)
    }

    #[inline(always)]
    pub fn reset(&mut self, other: Self) -> Result<(), ProtoError> {
        match (&self.kind, &other.kind) {
            (Kind::Chunked(..), Kind::Length(..)) | (Kind::Length(..), Kind::Chunked(..)) => {
                Err(ProtoError::Parse(Parse::Header))
            }
            _ => {
                *self = other;
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// A Reader used when a Content-Length header is passed with a positive
    /// integer.
    Length(u64),
    /// A Reader used when Transfer-Encoding is `chunked`.
    Chunked(ChunkedState, u64),
    /// A Reader used for responses that don't indicate a length or chunked.
    ///
    /// Note: This should only used for `Response`s. It is illegal for a
    /// `Request` to be made with both `Content-Length` and
    /// `Transfer-Encoding: chunked` missing, as explained from the spec:
    ///
    /// > If a Transfer-Encoding header field is present in a response and
    /// > the chunked transfer coding is not the final encoding, the
    /// > message body length is determined by reading the connection until
    /// > it is closed by the server.  If a Transfer-Encoding header field
    /// > is present in a request and the chunked transfer coding is not
    /// > the final encoding, the message body length cannot be determined
    /// > reliably; the server MUST respond with the 400 (Bad Request)
    /// > status code and then close the connection.
    Eof,
    /// A Reader function similar to Eof but treated as Chunked variant.
    /// The chunk is consumed directly as IS without actual decoding.
    /// This is used for upgrade type connections like websocket.
    PlainChunked,
}

#[derive(Debug, PartialEq, Clone)]
enum ChunkedState {
    Size,
    SizeLws,
    Extension,
    SizeLf,
    Body,
    BodyCr,
    BodyLf,
    EndCr,
    EndLf,
    End,
}

#[derive(Debug, Clone, PartialEq)]
/// Http payload item
pub enum RequestBodyItem {
    Chunk(Bytes),
    Eof,
}

impl RequestBodyDecoder {
    pub(super) fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<RequestBodyItem>> {
        match self.kind {
            Kind::Length(ref mut remaining) => {
                if *remaining == 0 {
                    Ok(Some(RequestBodyItem::Eof))
                } else {
                    if src.is_empty() {
                        return Ok(None);
                    }
                    let len = src.len() as u64;
                    let buf;
                    if *remaining > len {
                        buf = src.split().freeze();
                        *remaining -= len;
                    } else {
                        buf = src.split_to(*remaining as usize).freeze();
                        *remaining = 0;
                    };
                    Ok(Some(RequestBodyItem::Chunk(buf)))
                }
            }
            Kind::Chunked(ref mut state, ref mut size) => {
                loop {
                    let mut buf = None;
                    // advances the chunked state
                    *state = match state.step(src, size, &mut buf) {
                        Poll::Pending => return Ok(None),
                        Poll::Ready(Ok(state)) => state,
                        Poll::Ready(Err(e)) => return Err(e),
                    };
                    if *state == ChunkedState::End {
                        return Ok(Some(RequestBodyItem::Eof));
                    }
                    if let Some(buf) = buf {
                        return Ok(Some(RequestBodyItem::Chunk(buf)));
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
                    Ok(Some(RequestBodyItem::Chunk(src.split().freeze())))
                }
            }
        }
    }
}

macro_rules! byte (
    ($rdr:ident) => ({
        if $rdr.len() > 0 {
            let b = $rdr[0];
            $rdr.advance(1);
            b
        } else {
            return Poll::Pending
        }
    })
);

impl ChunkedState {
    fn step(&self, body: &mut BytesMut, size: &mut u64, buf: &mut Option<Bytes>) -> Poll<io::Result<ChunkedState>> {
        use self::ChunkedState::*;
        match *self {
            Size => ChunkedState::read_size(body, size),
            SizeLws => ChunkedState::read_size_lws(body),
            Extension => ChunkedState::read_extension(body),
            SizeLf => ChunkedState::read_size_lf(body, size),
            Body => ChunkedState::read_body(body, size, buf),
            BodyCr => ChunkedState::read_body_cr(body),
            BodyLf => ChunkedState::read_body_lf(body),
            EndCr => ChunkedState::read_end_cr(body),
            EndLf => ChunkedState::read_end_lf(body),
            End => Poll::Ready(Ok(ChunkedState::End)),
        }
    }

    fn read_size(rdr: &mut BytesMut, size: &mut u64) -> Poll<io::Result<ChunkedState>> {
        let radix = 16;
        match byte!(rdr) {
            b @ b'0'..=b'9' => {
                *size *= radix;
                *size += u64::from(b - b'0');
            }
            b @ b'a'..=b'f' => {
                *size *= radix;
                *size += u64::from(b + 10 - b'a');
            }
            b @ b'A'..=b'F' => {
                *size *= radix;
                *size += u64::from(b + 10 - b'A');
            }
            b'\t' | b' ' => return Poll::Ready(Ok(ChunkedState::SizeLws)),
            b';' => return Poll::Ready(Ok(ChunkedState::Extension)),
            b'\r' => return Poll::Ready(Ok(ChunkedState::SizeLf)),
            _ => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid chunk size line: Invalid Size",
                )));
            }
        }
        Poll::Ready(Ok(ChunkedState::Size))
    }

    fn read_size_lws(rdr: &mut BytesMut) -> Poll<io::Result<ChunkedState>> {
        match byte!(rdr) {
            // LWS can follow the chunk size, but no more digits can come
            b'\t' | b' ' => Poll::Ready(Ok(ChunkedState::SizeLws)),
            b';' => Poll::Ready(Ok(ChunkedState::Extension)),
            b'\r' => Poll::Ready(Ok(ChunkedState::SizeLf)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk size linear white space",
            ))),
        }
    }

    fn read_extension(rdr: &mut BytesMut) -> Poll<io::Result<ChunkedState>> {
        match byte!(rdr) {
            b'\r' => Poll::Ready(Ok(ChunkedState::SizeLf)),
            _ => Poll::Ready(Ok(ChunkedState::Extension)), // no supported extensions
        }
    }

    fn read_size_lf(rdr: &mut BytesMut, size: &mut u64) -> Poll<io::Result<ChunkedState>> {
        match byte!(rdr) {
            b'\n' if *size > 0 => Poll::Ready(Ok(ChunkedState::Body)),
            b'\n' if *size == 0 => Poll::Ready(Ok(ChunkedState::EndCr)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk size LF",
            ))),
        }
    }

    fn read_body(rdr: &mut BytesMut, rem: &mut u64, buf: &mut Option<Bytes>) -> Poll<io::Result<ChunkedState>> {
        let len = rdr.len() as u64;
        if len == 0 {
            Poll::Ready(Ok(ChunkedState::Body))
        } else {
            let slice;
            if *rem > len {
                slice = rdr.split().freeze();
                *rem -= len;
            } else {
                slice = rdr.split_to(*rem as usize).freeze();
                *rem = 0;
            }
            *buf = Some(slice);
            if *rem > 0 {
                Poll::Ready(Ok(ChunkedState::Body))
            } else {
                Poll::Ready(Ok(ChunkedState::BodyCr))
            }
        }
    }

    fn read_body_cr(rdr: &mut BytesMut) -> Poll<io::Result<ChunkedState>> {
        match byte!(rdr) {
            b'\r' => Poll::Ready(Ok(ChunkedState::BodyLf)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk body CR",
            ))),
        }
    }

    fn read_body_lf(rdr: &mut BytesMut) -> Poll<io::Result<ChunkedState>> {
        match byte!(rdr) {
            b'\n' => Poll::Ready(Ok(ChunkedState::Size)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk body LF",
            ))),
        }
    }

    fn read_end_cr(rdr: &mut BytesMut) -> Poll<io::Result<ChunkedState>> {
        match byte!(rdr) {
            b'\r' => Poll::Ready(Ok(ChunkedState::EndLf)),
            _ => Poll::Ready(Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end CR"))),
        }
    }

    fn read_end_lf(rdr: &mut BytesMut) -> Poll<io::Result<ChunkedState>> {
        match byte!(rdr) {
            b'\n' => Poll::Ready(Ok(ChunkedState::End)),
            _ => Poll::Ready(Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end LF"))),
        }
    }
}
