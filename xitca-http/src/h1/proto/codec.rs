use std::{io, task::Poll};

use bytes::{Buf, Bytes, BytesMut};
use tracing::trace;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Kind {
    /// Coder used when a Content-Length header is passed with a positive integer.
    Length(u64),

    /// Decoder used when Transfer-Encoding is `chunked`.
    DecodeChunked(ChunkedState, u64),

    /// Encoder for when Transfer-Encoding includes `chunked`.
    EncodeChunked(bool),

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
            return Poll::Pending
        }
    })
);

impl ChunkedState {
    pub(super) fn step(
        &self,
        body: &mut BytesMut,
        size: &mut u64,
        buf: &mut Option<Bytes>,
    ) -> Poll<io::Result<ChunkedState>> {
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

    fn read_trailer(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        trace!(target: "h1_decode", "read_trailer");
        match byte!(rdr) {
            b'\r' => Poll::Ready(Ok(ChunkedState::TrailerLf)),
            _ => Poll::Ready(Ok(ChunkedState::Trailer)),
        }
    }
    fn read_trailer_lf(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            b'\n' => Poll::Ready(Ok(ChunkedState::EndCr)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid trailer end LF",
            ))),
        }
    }

    fn read_end_cr(rdr: &mut BytesMut) -> Poll<io::Result<ChunkedState>> {
        match byte!(rdr) {
            b'\r' => Poll::Ready(Ok(ChunkedState::EndLf)),
            _ => Poll::Ready(Ok(ChunkedState::Trailer)),
        }
    }

    fn read_end_lf(rdr: &mut BytesMut) -> Poll<io::Result<ChunkedState>> {
        match byte!(rdr) {
            b'\n' => Poll::Ready(Ok(ChunkedState::End)),
            _ => Poll::Ready(Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end LF"))),
        }
    }
}
