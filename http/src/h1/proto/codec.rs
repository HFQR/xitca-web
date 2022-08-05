use std::io;

use tracing::{trace, warn};

use crate::bytes::{Buf, Bytes, BytesMut};

use super::{
    buf::BufWrite,
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
            Self::Eof | Self::Length(0) | Self::DecodeChunked(ChunkedState::End, _) => true,
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
    pub fn step(&self, body: &mut BytesMut, size: &mut u64, buf: &mut Option<Bytes>) -> io::Result<Option<Self>> {
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

    fn read_size_lf(rdr: &mut BytesMut, size: &mut u64) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\n' if *size > 0 => Ok(Some(Self::Body)),
            b'\n' if *size == 0 => Ok(Some(Self::EndCr)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk size LF")),
        }
    }

    fn read_body(rdr: &mut BytesMut, rem: &mut u64, buf: &mut Option<Bytes>) -> io::Result<Option<Self>> {
        let len = rdr.len() as u64;
        if len == 0 {
            Ok(Some(Self::Body))
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

    fn read_trailer(rdr: &mut BytesMut) -> io::Result<Option<Self>> {
        trace!(target: "h1_decode", "read_trailer");
        match byte!(rdr) {
            b'\r' => Ok(Some(Self::TrailerLf)),
            _ => Ok(Some(Self::Trailer)),
        }
    }

    fn read_trailer_lf(rdr: &mut BytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\n' => Ok(Some(Self::EndCr)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid trailer end LF")),
        }
    }

    fn read_end_cr(rdr: &mut BytesMut) -> io::Result<Option<Self>> {
        match byte!(rdr) {
            b'\r' => Ok(Some(Self::EndLf)),
            _ => Ok(Some(Self::Trailer)),
        }
    }

    fn read_end_lf(rdr: &mut BytesMut) -> io::Result<Option<Self>> {
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

    /// Encode message. Return `EOF` state of encoder
    pub fn encode<W>(&mut self, mut bytes: Bytes, buf: &mut W)
    where
        W: BufWrite,
    {
        // Skip encode empty bytes.
        // This is to avoid unnecessary extending on h1::proto::buf::ListBuf when user
        // provided empty bytes by accident.
        if bytes.is_empty() {
            return;
        }

        match *self {
            Self::Upgrade => buf.write_bytes(bytes),
            Self::EncodeChunked => buf.write_chunked(bytes),
            Self::Length(ref mut remaining) => {
                let len = bytes.len() as u64;
                if *remaining >= len {
                    buf.write_bytes(bytes);
                    *remaining -= len;
                } else {
                    buf.write_bytes(bytes.split_to(*remaining as usize));
                    *remaining = 0;
                }
            }
            Self::Eof => warn!(target: "h1_encode", "TransferCoding::Eof should not encode response body"),
            _ => unreachable!(),
        }
    }

    /// Encode eof. Return `EOF` state of encoder
    pub fn encode_eof<W>(&mut self, buf: &mut W)
    where
        W: BufWrite,
    {
        match *self {
            Self::Eof | Self::Upgrade | Self::Length(0) => {}
            Self::EncodeChunked => buf.write_static(b"0\r\n\r\n"),
            Self::Length(n) => unreachable!("UnexpectedEof for Length Body with {} remaining", n),
            _ => unreachable!(),
        }
    }

    /// decode body. return Some(Bytes) when successfully decoded new data.
    ///
    /// # Panics:
    /// Decode when Self is in EOF state would cause a panic. See [TransferCoding::is_eof] for.
    /// It's suggested to call `is_eof` before/after calling `decode` to make sure.
    pub fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Bytes>> {
        if src.is_empty() {
            return Ok(None);
        }

        match *self {
            Self::Eof | Self::Length(0) | Self::DecodeChunked(ChunkedState::End, _) => {
                unreachable!("TransferCoding::decode must not be called after it reaches EOF state. See Self::is_eof for condition.")
            }
            Self::Length(ref mut remaining) => {
                let len = src.len() as u64;
                let buf = if *remaining > len {
                    *remaining -= len;
                    src.split().freeze()
                } else {
                    let split = std::mem::replace(remaining, 0u64);
                    src.split_to(split as usize).freeze()
                };
                Ok(Some(buf))
            }
            Self::DecodeChunked(ref mut state, ref mut size) => {
                loop {
                    let mut buf = None;
                    // advances the chunked state
                    *state = match state.step(src, size, &mut buf)? {
                        Some(state) => state,
                        None => return Ok(None),
                    };

                    if src.is_empty() || matches!(state, ChunkedState::End) {
                        return Ok(None);
                    }

                    if let Some(buf) = buf {
                        return Ok(Some(buf));
                    }
                }
            }
            Self::Upgrade => Ok(Some(src.split().freeze())),
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::h1::proto::buf::FlatBuf;

    use super::*;

    #[test]
    fn test_read_chunk_size() {
        use std::io::ErrorKind::{InvalidData, InvalidInput, UnexpectedEof};

        fn read(s: &str) -> u64 {
            let mut state = ChunkedState::Size;
            let rdr = &mut BytesMut::from(s);
            let mut size = 0;
            loop {
                let result = state.step(rdr, &mut size, &mut None);
                state = result
                    .unwrap_or_else(|_| panic!("read_size failed for {:?}", s))
                    .unwrap();
                if state == ChunkedState::Body || state == ChunkedState::EndCr {
                    break;
                }
            }
            size
        }

        fn read_err(s: &str, expected_err: io::ErrorKind) {
            let mut state = ChunkedState::Size;
            let rdr = &mut BytesMut::from(s);
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
                    panic!("Was Ok. Expected Err for {:?}", s);
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
        let mock_buf = &mut BytesMut::from("10\r\n1234567890abcdef\r\n0\r\n");
        let buf = TransferCoding::decode_chunked().decode(mock_buf).unwrap().unwrap();
        assert_eq!(16, buf.len());
        let result = String::from_utf8(buf.as_ref().to_vec()).expect("decode String");
        assert_eq!("1234567890abcdef", &result);
    }

    #[test]
    fn test_read_chunked_trailer_with_missing_lf() {
        let mock_buf = &mut BytesMut::from("10\r\n1234567890abcdef\r\n0\r\nbad\r\r\n");
        let mut decoder = TransferCoding::decode_chunked();
        decoder.decode(mock_buf).unwrap().unwrap();
        let e = decoder.decode(mock_buf).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_read_chunked_after_eof() {
        let mock_buf = &mut BytesMut::from("10\r\n1234567890abcdef\r\n0\r\n\r\n");
        let mut decoder = TransferCoding::decode_chunked();

        // normal read
        let buf = decoder.decode(mock_buf).unwrap().unwrap();
        assert_eq!(16, buf.len());
        let result = String::from_utf8(buf.as_ref().to_vec()).unwrap();
        assert_eq!("1234567890abcdef", &result);

        // eof read
        assert!(decoder.decode(mock_buf).unwrap().is_none());

        // ensure eof state is observable.
        assert!(decoder.is_eof());
    }

    #[test]
    fn encode_chunked() {
        let mut encoder = TransferCoding::encode_chunked();
        let dst = &mut FlatBuf::<1024>::default();

        let msg1 = Bytes::from("foo bar");
        encoder.encode(msg1, dst);

        assert_eq!(&***dst, b"7\r\nfoo bar\r\n");

        let msg2 = Bytes::from("baz quux herp");
        encoder.encode(msg2, dst);

        assert_eq!(&***dst, b"7\r\nfoo bar\r\nD\r\nbaz quux herp\r\n");

        encoder.encode_eof(dst);

        assert_eq!(&***dst, b"7\r\nfoo bar\r\nD\r\nbaz quux herp\r\n0\r\n\r\n");
    }

    #[test]
    fn encode_length() {
        let max_len = 8;
        let mut encoder = TransferCoding::length(max_len as u64);

        let dst = &mut FlatBuf::<1024>::default();

        let msg1 = Bytes::from("foo bar");
        encoder.encode(msg1, dst);

        assert_eq!(&***dst, b"foo bar");

        for _ in 0..8 {
            let msg2 = Bytes::from("baz");
            encoder.encode(msg2, dst);

            assert_eq!(dst.len(), max_len);
            assert_eq!(&***dst, b"foo barb");
        }

        encoder.encode_eof(dst);
        assert_eq!(dst.len(), max_len);
        assert_eq!(&***dst, b"foo barb");
    }
}
