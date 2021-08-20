use std::io;

use bytes::{Buf, Bytes, BytesMut};
use tracing::trace;

/// Coder for different Transfer-Decoding/Transfer-Encoding.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum TransferCoding {
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
    pub(super) const fn eof() -> Self {
        Self::Eof
    }

    pub(super) fn is_eof(&self) -> bool {
        matches!(self, Self::Eof)
    }

    pub(super) const fn length(len: u64) -> Self {
        Self::Length(len)
    }

    pub(super) const fn decode_chunked() -> Self {
        Self::DecodeChunked(ChunkedState::Size, 0)
    }

    pub(super) const fn encode_chunked() -> Self {
        Self::EncodeChunked
    }

    pub(super) const fn plain_chunked() -> Self {
        Self::PlainChunked
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(super) enum ChunkedState {
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
    pub(super) fn step(
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
