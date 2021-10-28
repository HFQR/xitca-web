use std::{cmp, io};

use bytes::{Buf, Bytes, BytesMut};
use tracing::{trace, warn};

use super::{
    buf::WriteBuf,
    context::ConnectionType,
    error::{Parse, ProtoError},
};

/// Coder for different Transfer-Decoding/Transfer-Encoding.
#[derive(Debug, Clone, PartialEq)]
pub enum TransferCoding {
    /// Coder used when a Content-Length header is passed with a positive integer.
    Length(u64),

    /// Decoder used when Transfer-Encoding is `chunked`.
    DecodeChunked(ChunkedState, u64),

    /// Encoder for when Transfer-Encoding includes `chunked`.
    EncodeChunked,

    /// Coder used when coder that don't indicate a length or chunked.
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

    /// Coder function similar to Eof but treated as Chunked variant.
    /// The chunk is consumed directly as IS without actual decoding.
    /// This is used for upgrade type connections like websocket.
    PlainChunked,
}

impl TransferCoding {
    #[inline]
    pub const fn eof() -> Self {
        Self::Eof
    }

    #[inline]
    pub fn is_eof(&self) -> bool {
        matches!(self, Self::Eof)
    }

    #[inline]
    pub const fn length(len: u64) -> Self {
        Self::Length(len)
    }

    #[inline]
    pub const fn decode_chunked() -> Self {
        Self::DecodeChunked(ChunkedState::Size, 0)
    }

    #[inline]
    pub const fn encode_chunked() -> Self {
        Self::EncodeChunked
    }

    #[inline]
    pub const fn plain_chunked() -> Self {
        Self::PlainChunked
    }
}

#[derive(Debug, PartialEq, Clone)]
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
            return Ok(None)
        }
    })
);

impl ChunkedState {
    pub fn step(
        &self,
        body: &mut BytesMut,
        size: &mut u64,
        buf: &mut Option<Bytes>,
    ) -> io::Result<Option<ChunkedState>> {
        use self::ChunkedState::*;
        match *self {
            Size => ChunkedState::read_size(body, size),
            SizeLws => ChunkedState::read_size_lws(body),
            Extension => ChunkedState::read_extension(body),
            SizeLf => ChunkedState::read_size_lf(body, size),
            Body => ChunkedState::read_body(body, size, buf),
            BodyCr => ChunkedState::read_body_cr(body),
            BodyLf => ChunkedState::read_body_lf(body),
            Trailer => ChunkedState::read_trailer(body),
            TrailerLf => ChunkedState::read_trailer_lf(body),
            EndCr => ChunkedState::read_end_cr(body),
            EndLf => ChunkedState::read_end_lf(body),
            End => Ok(Some(ChunkedState::End)),
        }
    }

    fn read_size(rdr: &mut BytesMut, size: &mut u64) -> io::Result<Option<ChunkedState>> {
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

    fn read_size_lws(rdr: &mut BytesMut) -> io::Result<Option<ChunkedState>> {
        match byte!(rdr) {
            // LWS can follow the chunk size, but no more digits can come
            b'\t' | b' ' => Ok(Some(ChunkedState::SizeLws)),
            b';' => Ok(Some(ChunkedState::Extension)),
            b'\r' => Ok(Some(ChunkedState::SizeLf)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk size linear white space",
            )),
        }
    }

    fn read_extension(rdr: &mut BytesMut) -> io::Result<Option<ChunkedState>> {
        match byte!(rdr) {
            b'\r' => Ok(Some(ChunkedState::SizeLf)),
            b'\n' => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid chunk extension contains newline",
            )),
            _ => Ok(Some(ChunkedState::Extension)), // no supported extensions
        }
    }

    fn read_size_lf(rdr: &mut BytesMut, size: &mut u64) -> io::Result<Option<ChunkedState>> {
        match byte!(rdr) {
            b'\n' if *size > 0 => Ok(Some(ChunkedState::Body)),
            b'\n' if *size == 0 => Ok(Some(ChunkedState::EndCr)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk size LF")),
        }
    }

    fn read_body(rdr: &mut BytesMut, rem: &mut u64, buf: &mut Option<Bytes>) -> io::Result<Option<ChunkedState>> {
        let len = rdr.len() as u64;
        if len == 0 {
            Ok(Some(ChunkedState::Body))
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
                Ok(Some(ChunkedState::Body))
            } else {
                Ok(Some(ChunkedState::BodyCr))
            }
        }
    }

    fn read_body_cr(rdr: &mut BytesMut) -> io::Result<Option<ChunkedState>> {
        match byte!(rdr) {
            b'\r' => Ok(Some(ChunkedState::BodyLf)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body CR")),
        }
    }

    fn read_body_lf(rdr: &mut BytesMut) -> io::Result<Option<ChunkedState>> {
        match byte!(rdr) {
            b'\n' => Ok(Some(ChunkedState::Size)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body LF")),
        }
    }

    fn read_trailer(rdr: &mut BytesMut) -> io::Result<Option<ChunkedState>> {
        trace!(target: "h1_decode", "read_trailer");
        match byte!(rdr) {
            b'\r' => Ok(Some(ChunkedState::TrailerLf)),
            _ => Ok(Some(ChunkedState::Trailer)),
        }
    }
    fn read_trailer_lf(rdr: &mut BytesMut) -> io::Result<Option<ChunkedState>> {
        match byte!(rdr) {
            b'\n' => Ok(Some(ChunkedState::EndCr)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid trailer end LF")),
        }
    }

    fn read_end_cr(rdr: &mut BytesMut) -> io::Result<Option<ChunkedState>> {
        match byte!(rdr) {
            b'\r' => Ok(Some(ChunkedState::EndLf)),
            _ => Ok(Some(ChunkedState::Trailer)),
        }
    }

    fn read_end_lf(rdr: &mut BytesMut) -> io::Result<Option<ChunkedState>> {
        match byte!(rdr) {
            b'\n' => Ok(Some(ChunkedState::End)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end LF")),
        }
    }
}

impl TransferCoding {
    pub fn try_set(&mut self, other: Self) -> Result<(), ProtoError> {
        match (&self, &other) {
            // multiple set to plain chunked is allowed. This can happen from Connect method
            // and/or Connection header.
            (TransferCoding::PlainChunked, TransferCoding::PlainChunked) => Ok(()),
            // multiple set to decoded chunked/content-length are forbidden.
            //
            // mutation between decoded chunked/content-length/plain chunked is forbidden.
            (TransferCoding::PlainChunked, _)
            | (TransferCoding::DecodeChunked(..), _)
            | (TransferCoding::Length(..), _) => Err(ProtoError::Parse(Parse::HeaderName)),
            _ => {
                *self = other;
                Ok(())
            }
        }
    }

    pub(super) fn encode_chunked_from(ctype: ConnectionType) -> Self {
        if ctype == ConnectionType::Upgrade {
            Self::plain_chunked()
        } else {
            Self::encode_chunked()
        }
    }

    /// Encode message. Return `EOF` state of encoder
    pub fn encode<W>(&mut self, mut bytes: Bytes, buf: &mut W)
    where
        W: WriteBuf,
    {
        // Skip encode empty bytes.
        // This is to avoid unnecessary extending on h1::proto::buf::ListBuf when user
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
    pub fn encode_eof<W>(&mut self, buf: &mut W)
    where
        W: WriteBuf,
    {
        match *self {
            Self::Eof | Self::PlainChunked | Self::Length(0) => {}
            Self::EncodeChunked => buf.write_static(b"0\r\n\r\n"),
            Self::Length(n) => unreachable!("UnexpectedEof for Length Body with {} remaining", n),
            _ => unreachable!(),
        }
    }

    pub fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Bytes>> {
        match *self {
            Self::Length(ref mut remaining) => {
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
            Self::DecodeChunked(ref mut state, ref mut size) => {
                loop {
                    let mut buf = None;
                    // advances the chunked state
                    *state = match state.step(src, size, &mut buf)? {
                        Some(state) => state,
                        None => return Ok(None),
                    };

                    if matches!(state, ChunkedState::End) {
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
            Self::PlainChunked => {
                if src.is_empty() {
                    Ok(None)
                } else {
                    // TODO: hyper split 8kb here instead of take all.
                    Ok(Some(src.split().freeze()))
                }
            }
            Self::Eof => unreachable!("TransferCoding::Eof must never attempt to decode request payload"),
            _ => unreachable!(),
        }
    }
}
