use std::{cmp, io, mem};

use tracing::{trace, warn};

use crate::bytes::{Buf, Bytes, BytesMut};

use super::{
    buf::WriteBuf,
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

    /// Upgrade type coding that pass through data as is without transforming.
    Upgrade,
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
    pub const fn upgrade() -> Self {
        Self::Upgrade
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
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF during chunk size line"));
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
            *rem = 0;
            Err(incomplete_body())
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
    #[inline]
    pub fn try_set(&mut self, other: Self) -> Result<(), ProtoError> {
        match (&self, &other) {
            // multiple set to upgrade is allowed. This can happen from Connect method
            // and/or Connection header.
            (TransferCoding::Upgrade, TransferCoding::Upgrade) => Ok(()),
            // multiple set to chunked/content-length are forbidden.
            // mutation between chunked/content-length/upgrade is forbidden.
            (TransferCoding::Upgrade, _)
            | (TransferCoding::DecodeChunked(..), _)
            | (TransferCoding::Length(..), _)
            | (TransferCoding::EncodeChunked, _) => Err(ProtoError::Parse(Parse::HeaderName)),
            _ => {
                *self = other;
                Ok(())
            }
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
            Self::Upgrade => buf.write_buf(bytes),
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
            Self::Eof | Self::Upgrade | Self::Length(0) => {}
            Self::EncodeChunked => buf.write_static(b"0\r\n\r\n"),
            Self::Length(n) => unreachable!("UnexpectedEof for Length Body with {} remaining", n),
            _ => unreachable!(),
        }
    }

    /// Encode body. Return Bytes when successfully encoded new data.
    ///
    /// When returned bytes has zero length it means the encoder should enter Eof state.
    /// (calling `Self::encode_eof`)
    pub fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Bytes>> {
        match *self {
            Self::Length(0) => Ok(Some(Bytes::new())),
            Self::Length(ref mut remaining) => {
                if src.is_empty() {
                    return Ok(None);
                }
                let len = src.len() as u64;
                let buf = if *remaining > len {
                    *remaining -= len;
                    src.split().freeze()
                } else {
                    let mut split = 0;
                    mem::swap(remaining, &mut split);
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

                    if matches!(state, ChunkedState::End) {
                        return Ok(Some(Bytes::new()));
                    }

                    if let Some(buf) = buf {
                        return Ok(Some(buf));
                    }
                }
            }
            Self::Upgrade => {
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

#[cold]
#[inline(never)]
fn incomplete_body() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "end of file before message length reached",
    )
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
                    Ok(s) => s.unwrap(),
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
    fn test_read_chunked_early_eof() {
        let bytes = &mut BytesMut::from(
            "\
            9\r\n\
            foo bar\
        ",
        );
        let mut decoder = TransferCoding::decode_chunked();
        let n = decoder.decode(bytes).unwrap().unwrap().len();
        assert_eq!(n, 7);
        let e = decoder.decode(bytes).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
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
        let buf = decoder.decode(mock_buf).unwrap().unwrap();
        assert_eq!(0, buf.len());

        // ensure read after eof also returns eof
        let buf = decoder.decode(mock_buf).unwrap().unwrap();
        assert_eq!(0, buf.len());
    }

    // TODO: Revive async tests.
    // // perform an async read using a custom buffer size and causing a blocking
    // // read at the specified byte
    // fn read_async(mut decoder: TransferCoding, content: &[u8], block_at: usize) -> String {
    //     let mut outs = Vec::new();
    //
    //     let mut ins = if block_at == 0 {
    //         tokio_test::io::Builder::new()
    //             .wait(Duration::from_millis(10))
    //             .read(content)
    //             .build()
    //     } else {
    //         tokio_test::io::Builder::new()
    //             .read(&content[..block_at])
    //             .wait(Duration::from_millis(10))
    //             .read(&content[block_at..])
    //             .build()
    //     };
    //
    //     let mut ins = &mut ins as &mut (dyn AsyncRead + Unpin);
    //
    //     loop {
    //         let buf = decoder
    //             .decode_fut(&mut ins)
    //             .await
    //             .expect("unexpected decode error");
    //         if buf.is_empty() {
    //             break; // eof
    //         }
    //         outs.extend(buf.as_ref());
    //     }
    //
    //     String::from_utf8(outs).unwrap()
    // }
    //
    // // iterate over the different ways that this async read could go.
    // // tests blocking a read at each byte along the content - The shotgun approach
    // async fn all_async_cases(content: &str, expected: &str, decoder: Decoder) {
    //     let content_len = content.len();
    //     for block_at in 0..content_len {
    //         let actual = read_async(decoder.clone(), content.as_bytes(), block_at).await;
    //         assert_eq!(expected, &actual) //, "Failed async. Blocking at {}", block_at);
    //     }
    // }
    //
    // #[tokio::test]
    // async fn test_read_length_async() {
    //     let content = "foobar";
    //     all_async_cases(content, content, Decoder::length(content.len() as u64)).await;
    // }
    //
    // #[tokio::test]
    // async fn test_read_chunked_async() {
    //     let content = "3\r\nfoo\r\n3\r\nbar\r\n0\r\n\r\n";
    //     let expected = "foobar";
    //     all_async_cases(content, expected, Decoder::chunked()).await;
    // }
    //
    // #[tokio::test]
    // async fn test_read_eof_async() {
    //     let content = "foobar";
    //     all_async_cases(content, content, Decoder::eof()).await;
    // }

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
