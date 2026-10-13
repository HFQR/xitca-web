use super::*;

#[test]
fn test_peek_u8() {
    let b = 0xff;
    let mut buf = Cursor::new(vec![b]);
    assert_eq!(peek_u8(&buf), Some(b));
    assert_eq!(buf.get_u8(), b);
    assert_eq!(peek_u8(&buf), None);
}

#[test]
fn test_decode_string_empty() {
    let mut de = Decoder::new(0);
    let mut buf = BytesMut::new();
    let err = de.decode_string(&mut Cursor::new(&mut buf)).unwrap_err();
    assert_eq!(err, DecoderError::NeedMore(NeedMore::UnexpectedEndOfStream));
}

#[test]
fn test_decode_empty() {
    let mut de = Decoder::new(0);
    let mut buf = BytesMut::new();
    de.decode(&mut Cursor::new(&mut buf), |_| {}).unwrap();
}

#[test]
fn test_decode_indexed_larger_than_table() {
    let mut de = Decoder::new(0);

    let mut buf = BytesMut::new();
    buf.extend([0b01000000, 0x80 | 2]);
    buf.extend(huff_encode(b"foo"));
    buf.extend([0x80 | 3]);
    buf.extend(huff_encode(b"bar"));

    let mut res = vec![];
    de.decode(&mut Cursor::new(&mut buf), |h| {
        res.push(h);
    })
    .unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(de.table.size, 0);

    match res[0] {
        Header::Field { ref name, ref value } => {
            assert_eq!(name, "foo");
            assert_eq!(value, "bar");
        }
        _ => panic!(),
    }
}

fn huff_encode(src: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    huffman::encode(src, &mut buf);
    buf
}

#[test]
fn test_decode_continuation_header_with_non_huff_encoded_name() {
    let mut de = Decoder::new(0);
    let value = huff_encode(b"bar");
    let mut buf = BytesMut::new();
    // header name is non_huff encoded
    buf.extend([0b01000000, 3]);
    buf.extend(b"foo");
    // header value is partial
    buf.extend([0x80 | 3]);
    buf.extend(&value[0..1]);

    let mut res = vec![];
    let e = de
        .decode(&mut Cursor::new(&mut buf), |h| {
            res.push(h);
        })
        .unwrap_err();
    // decode error because the header value is partial
    assert_eq!(e, DecoderError::NeedMore(NeedMore::StringUnderflow));

    // extend buf with the remaining header value
    buf.extend(&value[1..]);
    de.decode(&mut Cursor::new(&mut buf), |h| {
        res.push(h);
    })
    .unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(de.table.size, 0);

    match res[0] {
        Header::Field { ref name, ref value } => {
            assert_eq!(name, "foo");
            assert_eq!(value, "bar");
        }
        _ => panic!(),
    }
}
