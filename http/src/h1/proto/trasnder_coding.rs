use core::{fmt, mem};

use std::io;

use tracing::{trace, warn};

use crate::{
    bytes::{Buf, BufMut, Bytes, BytesMut},
    http::header::{HeaderMap, HeaderName, HeaderValue},
};

use super::{buf_write::H1BufWrite, error::ProtoError};

/// Maximum number of bytes allowed for all trailer fields.
const TRAILER_MAX_HEADER_SIZE: usize = 1024 * 16;

/// Coder for different Transfer-Decoding/Transfer-Encoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransferCoding {
    /// Default coder indicates the Request/Response have successfully code it's associated body.
    Eof,
    /// Corrupted coder that can not be used anymore.
    Corrupted,
    /// Coder used when a Content-Length header is passed with a positive integer.
    Length(u64),
    /// Decoder used when Transfer-Encoding is `chunked`.
    DecodeChunked {
        state: ChunkedState,
        size: u64,
        trailers: Trailers,
    },
    /// Encoder for when Transfer-Encoding includes `chunked`.
    EncodeChunked,
    /// Upgrade type coder that pass through body as is without transforming.
    Upgrade,
    /// Decoder for HTTP/1.0-style responses where the body is delimited by
    /// connection closure (no Content-Length, no Transfer-Encoding, Connection: close).
    CloseDelimited,
}

/// State for accumulating chunked trailer headers during decode.
/// The `buf` is lazily boxed only when trailers are actually present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Trailers {
    buf: Option<Box<BytesMut>>,
    len: usize,
    limit: usize,
    size_limit: usize,
}

impl Trailers {
    // TODO: expose size_limit to HttpServiceConfig
    pub fn new(header_limit: usize) -> Self {
        Self {
            buf: None,
            len: 0,
            limit: header_limit,
            size_limit: TRAILER_MAX_HEADER_SIZE,
        }
    }

    fn try_put(&mut self, byte: u8) -> io::Result<()> {
        if self.buf.is_some() {
            self.put(byte)?;
        }
        Ok(())
    }

    fn put(&mut self, byte: u8) -> io::Result<()> {
        if let Some(buf) = self.buf.as_deref_mut() {
            buf.put_u8(byte);
            if buf.len() > self.size_limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "chunk trailers bytes over limit",
                ));
            }
        } else {
            let mut buf = BytesMut::with_capacity(64);
            buf.put_u8(byte);
            self.buf = Some(Box::new(buf));
        };

        Ok(())
    }

    fn incr_len(&mut self) -> io::Result<()> {
        self.len += 1;
        if self.len > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk trailers count overflow",
            ));
        }
        Ok(())
    }

    fn take(&mut self) -> Option<Self> {
        self.buf.is_some().then(|| mem::replace(self, Trailers::new(0)))
    }

    fn decode(self) -> io::Result<HeaderMap> {
        let buf = self.buf.expect("trailer buf must be initialized");
        let mut headers = vec![httparse::EMPTY_HEADER; self.len];
        match httparse::parse_headers(&buf, &mut headers) {
            Ok(httparse::Status::Complete((_, parsed))) => {
                let mut map = HeaderMap::with_capacity(parsed.len());
                for header in parsed {
                    let name = HeaderName::from_bytes(header.name.as_bytes())
                        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid trailer header name"))?;
                    let value = HeaderValue::from_bytes(header.value)
                        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid trailer header value"))?;
                    map.append(name, value);
                }
                Ok(map)
            }
            Ok(httparse::Status::Partial) => {
                Err(io::Error::new(io::ErrorKind::InvalidInput, "partial trailer headers"))
            }
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidInput, e)),
        }
    }
}

impl TransferCoding {
    #[inline]
    pub const fn eof() -> Self {
        Self::Eof
    }

    #[inline]
    pub const fn length(len: u64) -> Self {
        Self::Length(len)
    }

    #[inline]
    pub fn decode_chunked(header_limit: usize) -> Self {
        Self::DecodeChunked {
            state: ChunkedState::Size,
            size: 0,
            trailers: Trailers::new(header_limit),
        }
    }

    #[inline]
    pub const fn encode_chunked() -> Self {
        Self::EncodeChunked
    }

    #[inline]
    pub const fn upgrade() -> Self {
        Self::Upgrade
    }

    #[inline]
    pub const fn close_delimited() -> Self {
        Self::CloseDelimited
    }

    /// Check if Self is in EOF state. An EOF state means TransferCoding is ended gracefully
    /// and can not decode any value. See [TransferCoding::decode] for detail.
    #[inline]
    pub fn is_eof(&self) -> bool {
        match self {
            Self::Eof => true,
            Self::EncodeChunked => unreachable!("TransferCoding can't decide eof state when encoding chunked data"),
            _ => false,
        }
    }

    #[inline]
    pub fn is_upgrade(&self) -> bool {
        matches!(self, Self::Upgrade)
    }

    #[inline]
    pub fn is_close_delimited(&self) -> bool {
        matches!(self, Self::CloseDelimited)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChunkedState {
    Size,
    SizeLws,
    Extension,
    SizeLf,
    Body,
    BodyCr,
    BodyLf,
    Trailer,
    TrailerLf,
    EndCr,
    EndLf,
    End,
}

macro_rules! byte (
    ($rdr:ident) => ({
        if $rdr.len() > 0 {
            let b = $rdr[0];
            $rdr.advance(1);
            b
        } else {
            return Ok(None);
        }
    })
);

impl ChunkedState {
    pub fn step(
        &mut self,
        body: &mut BytesMut,
        size: &mut u64,
        buf: &mut Option<Bytes>,
        trailers: &mut Trailers,
    ) -> io::Result<Option<Self>> {
        match *self {
            Self::Size => Self::read_size(body, size),
            Self::SizeLws => Self::read_size_lws(body),
            Self::Extension => Self::read_extension(body),
            Self::SizeLf => Self::read_size_lf(body, size),
            Self::Body => Self::read_body(body, size, buf),
            Self::BodyCr => Self::read_body_cr(body),
            Self::BodyLf => Self::read_body_lf(body),
            Self::Trailer => Self::read_trailer(body, trailers),
            Self::TrailerLf => Self::read_trailer_lf(body, trailers),
            Self::EndCr => Self::read_end_cr(body, trailers),
            Self::EndLf => Self::read_end_lf(body, trailers),
            Self::End => Ok(Some(Self::End)),
        }
    }

    fn read_size(rdr: &mut BytesMut, size: &mut u64) -> io::Result<Option<Self>> {
        macro_rules! or_overflow {
            ($e:expr) => (
                match $e {
                    Some(val) => val,
                    None => return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid chunk size: overflow",
                    )),
                }
            )
        }

        let radix = 16;
        match byte!(rdr) {
            b @ b'0'..=b'9' => {
                *size = or_overflow!(size.checked_mul(radix));
                *size = or_overflow!(size.checked_add((b - b'0') as u64));
            }
            b @ b'a'..=b'f' => {
                *size = or_overflow!(size.checked_mul(radix));
                *size = or_overflow!(size.checked_add((b + 10 - b'a') as u64));
            }
            b @ b'A'..=b'F' => {
                *size = or_overflow!(size.checked_mul(radix));
                *size = or_overflow!(size.checked_add((b + 10 - b'A') as u64));
            }
            b'\t' | b' ' => return Ok(Some(ChunkedState::SizeLws)),
            b';' => return Ok(Some(ChunkedState::Extension)),
            b'\r' => return Ok(Some(ChunkedState::SizeLf)),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid chunk size line: Invalid Size",
                ));
            }
        }

        Ok(Some(ChunkedState::Size))
    }

    fn read_size_lws(rdr: &mut BytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            // LWS can follow the chunk size, but no more digits can come
            b'\t' | b' ' => Ok(Some(Self::SizeLws)),
            b';' => Ok(Some(Self::Extension)),
            b'\r' => Ok(Some(Self::SizeLf)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk size linear white space",
            )),
        }
    }

    fn read_extension(rdr: &mut BytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\r' => Ok(Some(Self::SizeLf)),
            b'\n' => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid chunk extension contains newline",
            )),
            _ => Ok(Some(Self::Extension)), // no supported extensions
        }
    }

    fn read_size_lf(rdr: &mut BytesMut, size: &u64) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\n' if *size > 0 => Ok(Some(Self::Body)),
            b'\n' if *size == 0 => Ok(Some(Self::EndCr)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk size LF")),
        }
    }

    fn read_body(rdr: &mut BytesMut, rem: &mut u64, buf: &mut Option<Bytes>) -> io::Result<Option<Self>> {
        if rdr.is_empty() {
            Ok(None)
        } else {
            *buf = Some(bounded_split(rem, rdr));
            if *rem > 0 {
                Ok(Some(Self::Body))
            } else {
                Ok(Some(Self::BodyCr))
            }
        }
    }

    fn read_body_cr(rdr: &mut BytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\r' => Ok(Some(Self::BodyLf)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body CR")),
        }
    }

    fn read_body_lf(rdr: &mut BytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\n' => Ok(Some(Self::Size)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body LF")),
        }
    }

    fn read_trailer(rdr: &mut BytesMut, trailers: &mut Trailers) -> io::Result<Option<Self>> {
        trace!(target: "h1_decode", "read_trailer");
        let byte = byte!(rdr);
        trailers.put(byte)?;
        match byte {
            b'\r' => Ok(Some(Self::TrailerLf)),
            _ => Ok(Some(Self::Trailer)),
        }
    }

    fn read_trailer_lf(rdr: &mut BytesMut, trailers: &mut Trailers) -> io::Result<Option<Self>> {
        let byte = byte!(rdr);
        match byte {
            b'\n' => {
                trailers.incr_len()?;
                trailers.put(byte)?;
                Ok(Some(Self::EndCr))
            }
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid trailer end LF")),
        }
    }

    fn read_end_cr(rdr: &mut BytesMut, trailers: &mut Trailers) -> io::Result<Option<Self>> {
        let byte = byte!(rdr);
        match byte {
            b'\r' => {
                trailers.try_put(byte)?;
                Ok(Some(Self::EndLf))
            }
            _ => {
                trailers.put(byte)?;
                Ok(Some(Self::Trailer))
            }
        }
    }

    fn read_end_lf(rdr: &mut BytesMut, trailers: &mut Trailers) -> io::Result<Option<Self>> {
        let byte = byte!(rdr);
        match byte {
            b'\n' => {
                trailers.try_put(byte)?;
                Ok(Some(Self::End))
            }
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end LF")),
        }
    }
}

impl TransferCoding {
    pub fn try_set(&mut self, other: Self) -> Result<(), ProtoError> {
        match (&self, &other) {
            // multiple set to plain chunked is allowed. This can happen from Connect method
            // and/or Connection header.
            // skip set when the request body is zero length.
            (TransferCoding::Upgrade, TransferCoding::Upgrade) | (_, TransferCoding::Length(0)) => Ok(()),
            // multiple set to decoded chunked/content-length are forbidden.
            // mutation between decoded chunked/content-length/plain chunked is forbidden.
            (TransferCoding::Upgrade, _)
            | (TransferCoding::DecodeChunked { .. }, _)
            | (TransferCoding::Length(..), _) => Err(ProtoError::HeaderName),
            _ => {
                *self = other;
                Ok(())
            }
        }
    }

    #[inline]
    pub fn set_eof(&mut self) {
        *self = Self::Eof;
    }

    #[inline]
    pub fn set_corrupted(&mut self) {
        *self = Self::Corrupted;
    }

    /// Encode message. Return `EOF` state of encoder
    pub fn encode<W>(&mut self, mut bytes: Bytes, buf: &mut W)
    where
        W: H1BufWrite,
    {
        // Skip encode empty bytes.
        // This is to avoid unnecessary extending on h1::proto::buf::ListBuf when user
        // provided empty bytes by accident.
        if bytes.is_empty() {
            return;
        }

        match *self {
            Self::Upgrade | Self::CloseDelimited => buf.write_buf_bytes(bytes),
            Self::EncodeChunked => buf.write_buf_bytes_chunked(bytes),
            Self::Length(ref mut rem) => {
                let len = bytes.len() as u64;
                if *rem >= len {
                    buf.write_buf_bytes(bytes);
                    *rem -= len;
                } else {
                    let rem = mem::replace(rem, 0u64);
                    buf.write_buf_bytes(bytes.split_to(rem as usize));
                }
            }
            Self::Eof => warn!(target: "h1_encode", "TransferCoding::Eof should not encode response body"),
            _ => unreachable!(),
        }
    }

    /// Encode eof with optional trailer headers.
    ///
    /// For chunked encoding, forbidden trailer fields (per RFC 9110) are silently filtered out.
    pub fn encode_eof<W>(&mut self, trailers: Option<HeaderMap>, buf: &mut W)
    where
        W: H1BufWrite,
    {
        match *self {
            Self::Eof | Self::Upgrade | Self::CloseDelimited | Self::Length(0) => {}
            Self::EncodeChunked => match trailers {
                Some(trailers) => buf.write_buf_trailers(trailers),
                None => buf.write_buf_static(b"0\r\n\r\n"),
            },
            Self::Length(n) => unreachable!("UnexpectedEof for Length Body with {} remaining", n),
            _ => unreachable!(),
        }
    }

    /// decode body. See [ChunkResult] for detailed outcome.
    pub fn decode(&mut self, src: &mut BytesMut) -> ChunkResult {
        match *self {
            // when decoder reaching eof state it would return ChunkResult::Eof and followed by
            // ChunkResult::AlreadyEof if decode is called again.
            // This multi stage behaviour is depended on by the caller to know the exact timing of
            // when eof happens. (Expensive one time operations can be happening at Eof)
            Self::Length(0)
            | Self::DecodeChunked {
                state: ChunkedState::End,
                ..
            } => {
                *self = Self::Eof;
                ChunkResult::OnEof
            }
            Self::Eof => ChunkResult::AlreadyEof,
            Self::Corrupted => ChunkResult::Corrupted,
            ref _this if src.is_empty() => ChunkResult::InsufficientData,
            Self::Length(ref mut rem) => ChunkResult::Ok(bounded_split(rem, src)),
            Self::Upgrade | Self::CloseDelimited => ChunkResult::Ok(src.split().freeze()),
            Self::DecodeChunked {
                ref mut state,
                ref mut size,
                ref mut trailers,
            } => {
                loop {
                    let mut buf = None;
                    // advances the chunked state
                    *state = match state.step(src, size, &mut buf, trailers) {
                        Ok(Some(state)) => state,
                        Ok(None) => return ChunkResult::InsufficientData,
                        Err(e) => return ChunkResult::Err(e),
                    };

                    if matches!(state, ChunkedState::End) {
                        if let Some(trailers) = trailers.take() {
                            match trailers.decode() {
                                Ok(headers) => return ChunkResult::Trailers(headers),
                                Err(e) => return ChunkResult::Err(e),
                            }
                        }
                        return self.decode(src);
                    }

                    if let Some(buf) = buf {
                        return ChunkResult::Ok(buf);
                    }
                }
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
pub enum ChunkResult {
    /// non empty chunk data produced by coder.
    Ok(Bytes),
    /// trailer headers decoded from chunked transfer encoding.
    Trailers(HeaderMap),
    /// io error type produced by coder that can be bubbled up to upstream caller.
    Err(io::Error),
    /// insufficient data. More input bytes required.
    InsufficientData,
    /// coder reached EOF state and no more chunk can be produced.
    OnEof,
    /// coder already reached EOF state and no more chunk can be produced.
    /// used to hint calling stop filling input buffer with more data and/or calling method again.
    AlreadyEof,
    /// see [TransferCoding::Corrupted].
    Corrupted,
}

impl fmt::Display for ChunkResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Ok(_) => f.write_str("chunked data."),
            Self::Trailers(_) => f.write_str("trailer headers."),
            Self::Err(ref e) => fmt::Display::fmt(e, f),
            Self::InsufficientData => f.write_str("no sufficient data. More input bytes required."),
            Self::OnEof => f.write_str("coder reached EOF state. no more chunk can be produced."),
            Self::AlreadyEof => f.write_str("coder already reached EOF state. no more chunk can be produced."),
            Self::Corrupted => f.write_str("coder corrupted. can not be used anymore."),
        }
    }
}

impl From<io::Error> for ChunkResult {
    fn from(e: io::Error) -> Self {
        Self::Err(e)
    }
}

fn bounded_split(rem: &mut u64, buf: &mut BytesMut) -> Bytes {
    let len = buf.len() as u64;
    if *rem >= len {
        *rem -= len;
        buf.split().freeze()
    } else {
        let rem = mem::replace(rem, 0);
        buf.split_to(rem as usize).freeze()
    }
}

#[cfg(test)]
mod tests;
