use core::{fmt, mem};

use std::io;

use tracing::{trace, warn};

use crate::{
    bytes::{Buf, Bytes},
    util::buffered::PagedBytesMut,
};

use super::{
    buf_write::H1BufWrite,
    error::{Parse, ProtoError},
};

/// Coder for different Transfer-Decoding/Transfer-Encoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransferCoding {
    /// Default coder indicates the Request/Response does not have a body.
    Eof,
    /// Coder used when a Content-Length header is passed with a positive integer.
    Length(u64),
    /// Decoder used when Transfer-Encoding is `chunked`.
    DecodeChunked(ChunkedState, u64),
    /// Encoder for when Transfer-Encoding includes `chunked`.
    EncodeChunked,
    /// Upgrade type coder that pass through body as is without transforming.
    Upgrade,
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
    pub const fn decode_chunked() -> Self {
        Self::DecodeChunked(ChunkedState::Size, 0)
    }

    #[inline]
    pub const fn encode_chunked() -> Self {
        Self::EncodeChunked
    }

    #[inline]
    pub const fn upgrade() -> Self {
        Self::Upgrade
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
        body: &mut PagedBytesMut,
        size: &mut u64,
        buf: &mut Option<Bytes>,
    ) -> io::Result<Option<Self>> {
        match *self {
            Self::Size => Self::read_size(body, size),
            Self::SizeLws => Self::read_size_lws(body),
            Self::Extension => Self::read_extension(body),
            Self::SizeLf => Self::read_size_lf(body, size),
            Self::Body => Self::read_body(body, size, buf),
            Self::BodyCr => Self::read_body_cr(body),
            Self::BodyLf => Self::read_body_lf(body),
            Self::Trailer => Self::read_trailer(body),
            Self::TrailerLf => Self::read_trailer_lf(body),
            Self::EndCr => Self::read_end_cr(body),
            Self::EndLf => Self::read_end_lf(body),
            Self::End => Ok(Some(Self::End)),
        }
    }

    fn read_size(rdr: &mut PagedBytesMut, size: &mut u64) -> io::Result<Option<Self>> {
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

    fn read_size_lws(rdr: &mut PagedBytesMut) -> io::Result<Option<Self>> {
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

    fn read_extension(rdr: &mut PagedBytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\r' => Ok(Some(Self::SizeLf)),
            b'\n' => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid chunk extension contains newline",
            )),
            _ => Ok(Some(Self::Extension)), // no supported extensions
        }
    }

    fn read_size_lf(rdr: &mut PagedBytesMut, size: &mut u64) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\n' if *size > 0 => Ok(Some(Self::Body)),
            b'\n' if *size == 0 => Ok(Some(Self::EndCr)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk size LF")),
        }
    }

    fn read_body(rdr: &mut PagedBytesMut, rem: &mut u64, buf: &mut Option<Bytes>) -> io::Result<Option<Self>> {
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

    fn read_body_cr(rdr: &mut PagedBytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\r' => Ok(Some(Self::BodyLf)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body CR")),
        }
    }

    fn read_body_lf(rdr: &mut PagedBytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\n' => Ok(Some(Self::Size)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body LF")),
        }
    }

    fn read_trailer(rdr: &mut PagedBytesMut) -> io::Result<Option<Self>> {
        trace!(target: "h1_decode", "read_trailer");
        match byte!(rdr) {
            b'\r' => Ok(Some(Self::TrailerLf)),
            _ => Ok(Some(Self::Trailer)),
        }
    }

    fn read_trailer_lf(rdr: &mut PagedBytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\n' => Ok(Some(Self::EndCr)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid trailer end LF")),
        }
    }

    fn read_end_cr(rdr: &mut PagedBytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\r' => Ok(Some(Self::EndLf)),
            _ => Ok(Some(Self::Trailer)),
        }
    }

    fn read_end_lf(rdr: &mut PagedBytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\n' => Ok(Some(Self::End)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end LF")),
        }
    }
}

impl TransferCoding {
    pub fn try_set(&mut self, other: Self) -> Result<(), ProtoError> {
        match (&self, &other) {
            // multiple set to plain chunked is allowed. This can happen from Connect method
            // and/or Connection header.
            (TransferCoding::Upgrade, TransferCoding::Upgrade) => Ok(()),
            // multiple set to decoded chunked/content-length are forbidden.
            //
            // mutation between decoded chunked/content-length/plain chunked is forbidden.
            (TransferCoding::Upgrade, _) | (TransferCoding::DecodeChunked(..), _) | (TransferCoding::Length(..), _) => {
                Err(ProtoError::Parse(Parse::HeaderName))
            }
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
            Self::Upgrade => buf.write_buf_bytes(bytes),
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

    /// Encode eof. Return `EOF` state of encoder
    pub fn encode_eof<W>(&mut self, buf: &mut W)
    where
        W: H1BufWrite,
    {
        match *self {
            Self::Eof | Self::Upgrade | Self::Length(0) => {}
            Self::EncodeChunked => buf.write_buf_static(b"0\r\n\r\n"),
            Self::Length(n) => unreachable!("UnexpectedEof for Length Body with {} remaining", n),
            _ => unreachable!(),
        }
    }

    /// decode body. See [ChunkResult] for detailed outcome.
    pub fn decode(&mut self, src: &mut PagedBytesMut) -> ChunkResult {
        match *self {
            // when decoder reaching eof state it would return ChunkResult::Eof and followed by
            // ChunkResult::AlreadyEof if decode is called again.
            // This multi stage behaviour is depended on by the caller to know the exact timing of
            // when eof happens. (Expensive one time operations can be happening at Eof)
            Self::Length(0) | Self::DecodeChunked(ChunkedState::End, _) => {
                *self = Self::Eof;
                ChunkResult::Eof
            }
            Self::Eof => ChunkResult::AlreadyEof,
            ref _this if src.is_empty() => ChunkResult::InsufficientData,
            Self::Length(ref mut rem) => ChunkResult::Ok(bounded_split(rem, src)),
            Self::Upgrade => ChunkResult::Ok(src.split().freeze()),
            Self::DecodeChunked(ref mut state, ref mut size) => {
                loop {
                    let mut buf = None;
                    // advances the chunked state
                    *state = match state.step(src, size, &mut buf) {
                        Ok(Some(state)) => state,
                        Ok(None) => return ChunkResult::InsufficientData,
                        Err(e) => return ChunkResult::Err(e),
                    };

                    if matches!(state, ChunkedState::End) {
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
    /// io error type produced by coder that can be bubbled up to upstream caller.
    Err(io::Error),
    /// insufficient data. More input bytes required.
    InsufficientData,
    /// coder reached EOF state and no more chunk can be produced.
    Eof,
    /// coder already reached EOF state and no more chunk can be produced.
    /// used to hint calling stop filling input buffer with more data and/or calling method again.
    AlreadyEof,
}

impl fmt::Display for ChunkResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Ok(_) => f.write_str("chunked data."),
            Self::InsufficientData => f.write_str("no sufficient data. More input bytes required."),
            Self::Eof => f.write_str("coder reached EOF state. no more chunk can be produced."),
            Self::AlreadyEof => f.write_str("coder already reached EOF state. no more chunk can be produced."),
            Self::Err(ref e) => fmt::Display::fmt(e, f),
        }
    }
}

impl From<io::Error> for ChunkResult {
    fn from(e: io::Error) -> Self {
        Self::Err(e)
    }
}

fn bounded_split(rem: &mut u64, buf: &mut PagedBytesMut) -> Bytes {
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
mod test {
    use crate::util::buffered::WriteBuf;

    use super::*;

    #[test]
    fn test_read_chunk_size() {
        use std::io::ErrorKind::{InvalidData, InvalidInput, UnexpectedEof};

        fn read(s: &str) -> u64 {
            let mut state = ChunkedState::Size;
            let rdr = &mut PagedBytesMut::from(s);
            let mut size = 0;
            loop {
                let result = state.step(rdr, &mut size, &mut None);
                state = result.unwrap_or_else(|_| panic!("read_size failed for {s:?}")).unwrap();
                if state == ChunkedState::Body || state == ChunkedState::EndCr {
                    break;
                }
            }
            size
        }

        fn read_err(s: &str, expected_err: io::ErrorKind) {
            let mut state = ChunkedState::Size;
            let rdr = &mut PagedBytesMut::from(s);
            let mut size = 0;
            loop {
                let result = state.step(rdr, &mut size, &mut None);
                state = match result {
                    Ok(Some(s)) => s,
                    Ok(None) => return assert_eq!(expected_err, UnexpectedEof),
                    Err(e) => {
                        assert_eq!(
                            expected_err,
                            e.kind(),
                            "Reading {:?}, expected {:?}, but got {:?}",
                            s,
                            expected_err,
                            e.kind()
                        );
                        return;
                    }
                };
                if state == ChunkedState::Body || state == ChunkedState::End {
                    panic!("Was Ok. Expected Err for {s:?}");
                }
            }
        }

        assert_eq!(1, read("1\r\n"));
        assert_eq!(1, read("01\r\n"));
        assert_eq!(0, read("0\r\n"));
        assert_eq!(0, read("00\r\n"));
        assert_eq!(10, read("A\r\n"));
        assert_eq!(10, read("a\r\n"));
        assert_eq!(255, read("Ff\r\n"));
        assert_eq!(255, read("Ff   \r\n"));
        // Missing LF or CRLF
        read_err("F\rF", InvalidInput);
        read_err("F", UnexpectedEof);
        // Invalid hex digit
        read_err("X\r\n", InvalidInput);
        read_err("1X\r\n", InvalidInput);
        read_err("-\r\n", InvalidInput);
        read_err("-1\r\n", InvalidInput);
        // Acceptable (if not fully valid) extensions do not influence the size
        assert_eq!(1, read("1;extension\r\n"));
        assert_eq!(10, read("a;ext name=value\r\n"));
        assert_eq!(1, read("1;extension;extension2\r\n"));
        assert_eq!(1, read("1;;;  ;\r\n"));
        assert_eq!(2, read("2; extension...\r\n"));
        assert_eq!(3, read("3   ; extension=123\r\n"));
        assert_eq!(3, read("3   ;\r\n"));
        assert_eq!(3, read("3   ;   \r\n"));
        // Invalid extensions cause an error
        read_err("1 invalid extension\r\n", InvalidInput);
        read_err("1 A\r\n", InvalidInput);
        read_err("1;no CRLF", UnexpectedEof);
        read_err("1;reject\nnewlines\r\n", InvalidData);
        // Overflow
        read_err("f0000000000000003\r\n", InvalidData);
    }

    #[test]
    fn test_read_chunked_single_read() {
        let mock_buf = &mut PagedBytesMut::from("10\r\n1234567890abcdef\r\n0\r\n");

        match TransferCoding::decode_chunked().decode(mock_buf) {
            ChunkResult::Ok(buf) => {
                assert_eq!(16, buf.len());
                let result = String::from_utf8(buf.as_ref().to_vec()).expect("decode String");
                assert_eq!("1234567890abcdef", &result);
            }
            state => panic!("{}", state),
        }
    }

    #[test]
    fn test_read_chunked_trailer_with_missing_lf() {
        let mock_buf = &mut PagedBytesMut::from("10\r\n1234567890abcdef\r\n0\r\nbad\r\r\n");

        let mut decoder = TransferCoding::decode_chunked();

        match decoder.decode(mock_buf) {
            ChunkResult::Ok(_) => {}
            state => panic!("{}", state),
        }

        match decoder.decode(mock_buf) {
            ChunkResult::Err(e) => assert_eq!(e.kind(), io::ErrorKind::InvalidInput),
            state => panic!("{}", state),
        }
    }

    #[test]
    fn test_read_chunked_after_eof() {
        let mock_buf = &mut PagedBytesMut::from("10\r\n1234567890abcdef\r\n0\r\n\r\n");
        let mut decoder = TransferCoding::decode_chunked();

        // normal read
        match decoder.decode(mock_buf) {
            ChunkResult::Ok(buf) => {
                assert_eq!(16, buf.len());
                let result = String::from_utf8(buf.as_ref().to_vec()).unwrap();
                assert_eq!("1234567890abcdef", &result);
            }
            state => panic!("{}", state),
        }

        // eof read
        match decoder.decode(mock_buf) {
            ChunkResult::Eof => {}
            state => panic!("{}", state),
        }

        // already meet eof
        match decoder.decode(mock_buf) {
            ChunkResult::AlreadyEof => {}
            state => panic!("{}", state),
        }
    }

    #[test]
    fn encode_chunked() {
        let mut encoder = TransferCoding::encode_chunked();
        let dst = &mut WriteBuf::<1024>::default();

        let msg1 = Bytes::from("foo bar");
        encoder.encode(msg1, dst);

        assert_eq!(dst.buf(), b"7\r\nfoo bar\r\n");

        let msg2 = Bytes::from("baz quux herp");
        encoder.encode(msg2, dst);

        assert_eq!(dst.buf(), b"7\r\nfoo bar\r\nD\r\nbaz quux herp\r\n");

        encoder.encode_eof(dst);

        assert_eq!(dst.buf(), b"7\r\nfoo bar\r\nD\r\nbaz quux herp\r\n0\r\n\r\n");
    }

    #[test]
    fn encode_length() {
        let max_len = 8;
        let mut encoder = TransferCoding::length(max_len as u64);

        let dst = &mut WriteBuf::<1024>::default();

        let msg1 = Bytes::from("foo bar");
        encoder.encode(msg1, dst);

        assert_eq!(dst.buf(), b"foo bar");

        for _ in 0..8 {
            let msg2 = Bytes::from("baz");
            encoder.encode(msg2, dst);

            assert_eq!(dst.buf().len(), max_len);
            assert_eq!(dst.buf(), b"foo barb");
        }

        encoder.encode_eof(dst);
        assert_eq!(dst.buf().len(), max_len);
        assert_eq!(dst.buf(), b"foo barb");
    }
}
