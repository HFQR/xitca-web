use super::*;

#[test]
fn test_read_chunk_size() {
    use std::io::ErrorKind::{InvalidData, InvalidInput, UnexpectedEof};

    fn read(s: &str) -> u64 {
        let mut state = ChunkedState::Size;
        let rdr = &mut BytesMut::from(s);
        let mut size = 0;
        loop {
            let result = state.step(rdr, &mut size, &mut None, &mut Trailers::new(64));
            state = result.unwrap_or_else(|_| panic!("read_size failed for {s:?}")).unwrap();
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
            let result = state.step(rdr, &mut size, &mut None, &mut Trailers::new(64));
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
    let mock_buf = &mut BytesMut::from("10\r\n1234567890abcdef\r\n0\r\n");

    match TransferCoding::decode_chunked(64).decode(mock_buf) {
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
    let mock_buf = &mut BytesMut::from("10\r\n1234567890abcdef\r\n0\r\nbad\r\r\n");

    let mut decoder = TransferCoding::decode_chunked(64);

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
    let mock_buf = &mut BytesMut::from("10\r\n1234567890abcdef\r\n0\r\n\r\n");
    let mut decoder = TransferCoding::decode_chunked(64);

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
        ChunkResult::OnEof => {}
        state => panic!("{}", state),
    }

    // already meet eof
    match decoder.decode(mock_buf) {
        ChunkResult::AlreadyEof => {}
        state => panic!("{}", state),
    }
}

#[test]
fn test_read_chunked_with_trailers() {
    // chunked body: "Hello" (5 bytes) followed by two trailer headers
    let mock_buf =
        &mut BytesMut::from("5\r\nHello\r\n0\r\nExpires: Wed, 21 Oct 2015 07:28:00 GMT\r\nX-Checksum: abc123\r\n\r\n");
    let mut decoder = TransferCoding::decode_chunked(64);

    // first decode yields the body data
    match decoder.decode(mock_buf) {
        ChunkResult::Ok(buf) => {
            assert_eq!(buf.as_ref(), b"Hello");
        }
        state => panic!("expected data chunk, got: {}", state),
    }

    // second decode yields the trailer headers
    match decoder.decode(mock_buf) {
        ChunkResult::Trailers(headers) => {
            assert_eq!(headers.len(), 2);
            assert_eq!(headers.get("Expires").unwrap(), "Wed, 21 Oct 2015 07:28:00 GMT");
            assert_eq!(headers.get("X-Checksum").unwrap(), "abc123");
        }
        state => panic!("expected trailers, got: {}", state),
    }

    // third decode yields eof
    match decoder.decode(mock_buf) {
        ChunkResult::OnEof => {}
        state => panic!("expected OnEof, got: {}", state),
    }
}

#[test]
fn encode_chunked() {
    let mut encoder = TransferCoding::encode_chunked();
    let dst = &mut BytesMut::default();

    let msg1 = Bytes::from("foo bar");
    encoder.encode(msg1, dst);

    assert_eq!(dst.as_ref(), b"7\r\nfoo bar\r\n");

    let msg2 = Bytes::from("baz quux herp");
    encoder.encode(msg2, dst);

    assert_eq!(dst.as_ref(), b"7\r\nfoo bar\r\nD\r\nbaz quux herp\r\n");

    encoder.encode_eof(None, dst);

    assert_eq!(dst.as_ref(), b"7\r\nfoo bar\r\nD\r\nbaz quux herp\r\n0\r\n\r\n");
}

#[test]
fn encode_length() {
    let max_len = 8;
    let mut encoder = TransferCoding::length(max_len as u64);

    let dst = &mut BytesMut::default();

    let msg1 = Bytes::from("foo bar");
    encoder.encode(msg1, dst);

    assert_eq!(dst.as_ref(), b"foo bar");

    for _ in 0..8 {
        let msg2 = Bytes::from("baz");
        encoder.encode(msg2, dst);

        assert_eq!(dst.as_ref().len(), max_len);
        assert_eq!(dst.as_ref(), b"foo barb");
    }

    encoder.encode_eof(None, dst);
    assert_eq!(dst.as_ref().len(), max_len);
    assert_eq!(dst.as_ref(), b"foo barb");
}

#[test]
fn encode_chunked_with_trailers() {
    let mut encoder = TransferCoding::encode_chunked();
    let dst = &mut BytesMut::default();

    let msg = Bytes::from("hello");
    encoder.encode(msg, dst);

    let mut trailers = HeaderMap::new();
    trailers.insert("x-checksum", HeaderValue::from_static("abc123"));
    trailers.insert("x-status", HeaderValue::from_static("ok"));
    trailers.append("x-status", HeaderValue::from_static("done"));

    encoder.encode_eof(Some(trailers), dst);

    assert_eq!(
        dst.as_ref(),
        b"5\r\nhello\r\n0\r\nx-checksum: abc123\r\nx-status: ok\r\nx-status: done\r\n\r\n"
    );
}

#[test]
fn encode_chunked_trailers_filters_forbidden() {
    use crate::http::header;

    let mut encoder = TransferCoding::encode_chunked();
    let dst = &mut BytesMut::default();

    let msg = Bytes::from("data");
    encoder.encode(msg, dst);

    let mut trailers = HeaderMap::new();
    trailers.insert("x-checksum", HeaderValue::from_static("abc123"));
    // forbidden fields per RFC 9110
    trailers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("99"));
    trailers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    trailers.insert(header::HOST, HeaderValue::from_static("example.com"));
    trailers.insert(header::TRANSFER_ENCODING, HeaderValue::from_static("chunked"));

    encoder.encode_eof(Some(trailers), dst);

    // only x-checksum should remain
    assert_eq!(dst.as_ref(), b"4\r\ndata\r\n0\r\nx-checksum: abc123\r\n\r\n");
}

#[test]
fn encode_chunked_trailers_empty_after_filter() {
    use crate::http::header;

    let mut encoder = TransferCoding::encode_chunked();
    let dst = &mut BytesMut::default();

    encoder.encode(Bytes::from("x"), dst);

    let mut trailers = HeaderMap::new();
    trailers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("1"));

    encoder.encode_eof(Some(trailers), dst);

    // all trailers filtered, should still produce valid chunked terminator
    assert_eq!(dst.as_ref(), b"1\r\nx\r\n0\r\n\r\n");
}
