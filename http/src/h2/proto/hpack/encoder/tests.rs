use http::*;

use super::*;

#[test]
fn test_encode_method_get() {
    let mut encoder = Encoder::default();
    let res = encode(&mut encoder, vec![method("GET")]);
    assert_eq!(*res, [0x80 | 2]);
    assert_eq!(encoder.table.len(), 0);
}

#[test]
fn test_encode_method_post() {
    let mut encoder = Encoder::default();
    let res = encode(&mut encoder, vec![method("POST")]);
    assert_eq!(*res, [0x80 | 3]);
    assert_eq!(encoder.table.len(), 0);
}

#[test]
fn test_encode_method_patch() {
    let mut encoder = Encoder::default();
    let res = encode(&mut encoder, vec![method("PATCH")]);

    assert_eq!(res[0], 0b01000000 | 2); // Incremental indexing w/ name pulled from table
    assert_eq!(res[1], 0x80 | 5); // header value w/ huffman coding

    assert_eq!("PATCH", huff_decode(&res[2..7]));
    assert_eq!(encoder.table.len(), 1);

    let res = encode(&mut encoder, vec![method("PATCH")]);

    assert_eq!((1 << 7) | 62, res[0]);
    assert_eq!(1, res.len());
}

#[test]
fn test_encode_indexed_name_literal_value() {
    let mut encoder = Encoder::default();
    let res = encode(&mut encoder, vec![header("content-language", "foo")]);

    assert_eq!(res[0], 0b01000000 | 27); // Indexed name
    assert_eq!(res[1], 0x80 | 2); // header value w/ huffman coding

    assert_eq!("foo", huff_decode(&res[2..4]));

    // Same name, new value should still use incremental
    let res = encode(&mut encoder, vec![header("content-language", "bar")]);
    assert_eq!(res[0], 0b01000000 | 27); // Indexed name
    assert_eq!(res[1], 0x80 | 3); // header value w/ huffman coding
    assert_eq!("bar", huff_decode(&res[2..5]));
}

#[test]
fn test_repeated_headers_are_indexed() {
    let mut encoder = Encoder::default();
    let res = encode(&mut encoder, vec![header("foo", "hello")]);

    assert_eq!(&[0b01000000, 0x80 | 2], &res[0..2]);
    assert_eq!("foo", huff_decode(&res[2..4]));
    assert_eq!(0x80 | 4, res[4]);
    assert_eq!("hello", huff_decode(&res[5..]));
    assert_eq!(9, res.len());

    assert_eq!(1, encoder.table.len());

    let res = encode(&mut encoder, vec![header("foo", "hello")]);
    assert_eq!([0x80 | 62], *res);

    assert_eq!(encoder.table.len(), 1);
}

#[test]
fn test_evicting_headers() {
    let mut encoder = Encoder::default();

    // Fill the table
    for i in 0..64 {
        let key = format!("x-hello-world-{i:02}");
        let res = encode(&mut encoder, vec![header(&key, &key)]);

        assert_eq!(&[0b01000000, 0x80 | 12], &res[0..2]);
        assert_eq!(key, huff_decode(&res[2..14]));
        assert_eq!(0x80 | 12, res[14]);
        assert_eq!(key, huff_decode(&res[15..]));
        assert_eq!(27, res.len());

        // Make sure the header can be found...
        let res = encode(&mut encoder, vec![header(&key, &key)]);

        // Only check that it is found
        assert_eq!(0x80, res[0] & 0x80);
    }

    assert_eq!(4096, encoder.table.size());
    assert_eq!(64, encoder.table.len());

    // Find existing headers
    for i in 0..64 {
        let key = format!("x-hello-world-{i:02}");
        let res = encode(&mut encoder, vec![header(&key, &key)]);
        assert_eq!(0x80, res[0] & 0x80);
    }

    // Insert a new header
    let key = "x-hello-world-64";
    let res = encode(&mut encoder, vec![header(key, key)]);

    assert_eq!(&[0b01000000, 0x80 | 12], &res[0..2]);
    assert_eq!(key, huff_decode(&res[2..14]));
    assert_eq!(0x80 | 12, res[14]);
    assert_eq!(key, huff_decode(&res[15..]));
    assert_eq!(27, res.len());

    assert_eq!(64, encoder.table.len());

    // Now try encoding entries that should exist in the table
    for i in 1..65 {
        let key = format!("x-hello-world-{i:02}");
        let res = encode(&mut encoder, vec![header(&key, &key)]);
        assert_eq!(0x80 | (61 + (65 - i)), res[0]);
    }
}

#[test]
fn test_large_headers_are_not_indexed() {
    let mut encoder = Encoder::new(128, 0);
    let key = "hello-world-hello-world-HELLO-zzz";

    let res = encode(&mut encoder, vec![header(key, key)]);

    assert_eq!(&[0, 0x80 | 25], &res[..2]);

    assert_eq!(0, encoder.table.len());
    assert_eq!(0, encoder.table.size());
}

#[test]
fn test_sensitive_headers_are_never_indexed() {
    use http::header::HeaderValue;

    let name = "my-password".parse().unwrap();
    let mut value = HeaderValue::from_bytes(b"12345").unwrap();
    value.set_sensitive(true);

    let header = Header::Field {
        name: Some(name),
        value,
    };

    // Now, try to encode the sensitive header

    let mut encoder = Encoder::default();
    let res = encode(&mut encoder, vec![header]);

    assert_eq!(&[0b10000, 0x80 | 8], &res[..2]);
    assert_eq!("my-password", huff_decode(&res[2..10]));
    assert_eq!(0x80 | 4, res[10]);
    assert_eq!("12345", huff_decode(&res[11..]));

    // Now, try to encode a sensitive header w/ a name in the static table
    let name = "authorization".parse().unwrap();
    let mut value = HeaderValue::from_bytes(b"12345").unwrap();
    value.set_sensitive(true);

    let header = Header::Field {
        name: Some(name),
        value,
    };

    let mut encoder = Encoder::default();
    let res = encode(&mut encoder, vec![header]);

    assert_eq!(&[0b11111, 8], &res[..2]);
    assert_eq!(0x80 | 4, res[2]);
    assert_eq!("12345", huff_decode(&res[3..]));

    // Using the name component of a previously indexed header (without
    // sensitive flag set)

    let _ = encode(&mut encoder, vec![self::header("my-password", "not-so-secret")]);

    let name = "my-password".parse().unwrap();
    let mut value = HeaderValue::from_bytes(b"12345").unwrap();
    value.set_sensitive(true);

    let header = Header::Field {
        name: Some(name),
        value,
    };
    let res = encode(&mut encoder, vec![header]);

    assert_eq!(&[0b11111, 47], &res[..2]);
    assert_eq!(0x80 | 4, res[2]);
    assert_eq!("12345", huff_decode(&res[3..]));
}

#[test]
fn test_content_length_value_not_indexed() {
    let mut encoder = Encoder::default();
    let res = encode(&mut encoder, vec![header("content-length", "1234")]);

    assert_eq!(&[15, 13, 0x80 | 3], &res[0..3]);
    assert_eq!("1234", huff_decode(&res[3..]));
    assert_eq!(6, res.len());
}

#[test]
fn test_encoding_headers_with_same_name() {
    let mut encoder = Encoder::default();
    let name = "hello";

    // Encode first one
    let _ = encode(&mut encoder, vec![header(name, "one")]);

    // Encode second one
    let res = encode(&mut encoder, vec![header(name, "two")]);
    assert_eq!(&[0x40 | 62, 0x80 | 3], &res[0..2]);
    assert_eq!("two", huff_decode(&res[2..]));
    assert_eq!(5, res.len());

    // Encode the first one again
    let res = encode(&mut encoder, vec![header(name, "one")]);
    assert_eq!(&[0x80 | 63], &res[..]);

    // Now the second one
    let res = encode(&mut encoder, vec![header(name, "two")]);
    assert_eq!(&[0x80 | 62], &res[..]);
}

#[test]
fn test_evicting_headers_when_multiple_of_same_name_are_in_table() {
    // The encoder only has space for 2 headers
    let mut encoder = Encoder::new(76, 0);

    let _ = encode(&mut encoder, vec![header("foo", "bar")]);
    assert_eq!(1, encoder.table.len());

    let _ = encode(&mut encoder, vec![header("bar", "foo")]);
    assert_eq!(2, encoder.table.len());

    // This will evict the first header, while still referencing the header
    // name
    let res = encode(&mut encoder, vec![header("foo", "baz")]);
    assert_eq!(&[0x40 | 63, 0, 0x80 | 3], &res[..3]);
    assert_eq!(2, encoder.table.len());

    // Try adding the same header again
    let res = encode(&mut encoder, vec![header("foo", "baz")]);
    assert_eq!(&[0x80 | 62], &res[..]);
    assert_eq!(2, encoder.table.len());
}

#[test]
fn test_max_size_zero() {
    // Static table only
    let mut encoder = Encoder::new(0, 0);
    let res = encode(&mut encoder, vec![method("GET")]);
    assert_eq!(*res, [0x80 | 2]);
    assert_eq!(encoder.table.len(), 0);

    let res = encode(&mut encoder, vec![header("foo", "bar")]);
    assert_eq!(&[0, 0x80 | 2], &res[..2]);
    assert_eq!("foo", huff_decode(&res[2..4]));
    assert_eq!(0x80 | 3, res[4]);
    assert_eq!("bar", huff_decode(&res[5..8]));
    assert_eq!(0, encoder.table.len());

    // Encode a custom value
    let res = encode(&mut encoder, vec![header("transfer-encoding", "chunked")]);
    assert_eq!(&[15, 42, 0x80 | 6], &res[..3]);
    assert_eq!("chunked", huff_decode(&res[3..]));
}

#[test]
fn test_update_max_size_combos() {
    let mut encoder = Encoder::default();
    assert!(encoder.size_update.is_none());
    assert_eq!(4096, encoder.table.max_size());

    encoder.update_max_size(4096); // Default size
    assert!(encoder.size_update.is_none());

    encoder.update_max_size(0);
    assert_eq!(Some(SizeUpdate::One(0)), encoder.size_update);

    encoder.update_max_size(100);
    assert_eq!(Some(SizeUpdate::Two(0, 100)), encoder.size_update);

    let mut encoder = Encoder::default();
    encoder.update_max_size(8000);
    assert_eq!(Some(SizeUpdate::One(8000)), encoder.size_update);

    encoder.update_max_size(100);
    assert_eq!(Some(SizeUpdate::One(100)), encoder.size_update);

    encoder.update_max_size(8000);
    assert_eq!(Some(SizeUpdate::Two(100, 8000)), encoder.size_update);

    encoder.update_max_size(4000);
    assert_eq!(Some(SizeUpdate::Two(100, 4000)), encoder.size_update);

    encoder.update_max_size(50);
    assert_eq!(Some(SizeUpdate::One(50)), encoder.size_update);
}

#[test]
fn test_resizing_table() {
    let mut encoder = Encoder::default();

    // Add a header
    let _ = encode(&mut encoder, vec![header("foo", "bar")]);

    encoder.update_max_size(1);
    assert_eq!(1, encoder.table.len());

    let res = encode(&mut encoder, vec![method("GET")]);
    assert_eq!(&[32 | 1, 0x80 | 2], &res[..]);
    assert_eq!(0, encoder.table.len());

    let res = encode(&mut encoder, vec![header("foo", "bar")]);
    assert_eq!(0, res[0]);

    encoder.update_max_size(100);
    let res = encode(&mut encoder, vec![header("foo", "bar")]);
    assert_eq!(&[32 | 31, 69, 64], &res[..3]);

    encoder.update_max_size(0);
    let res = encode(&mut encoder, vec![header("foo", "bar")]);
    assert_eq!(&[32, 0], &res[..2]);
}

#[test]
fn test_decreasing_table_size_without_eviction() {
    let mut encoder = Encoder::default();

    // Add a header
    let _ = encode(&mut encoder, vec![header("foo", "bar")]);

    encoder.update_max_size(100);
    assert_eq!(1, encoder.table.len());

    let res = encode(&mut encoder, vec![header("foo", "bar")]);
    assert_eq!(&[32 | 31, 69, 0x80 | 62], &res[..]);
}

#[test]
fn test_nameless_header() {
    let mut encoder = Encoder::default();

    let res = encode(
        &mut encoder,
        vec![
            Header::Field {
                name: Some("hello".parse().unwrap()),
                value: HeaderValue::from_bytes(b"world").unwrap(),
            },
            Header::Field {
                name: None,
                value: HeaderValue::from_bytes(b"zomg").unwrap(),
            },
        ],
    );

    assert_eq!(&[0x40, 0x80 | 4], &res[0..2]);
    assert_eq!("hello", huff_decode(&res[2..6]));
    assert_eq!(0x80 | 4, res[6]);
    assert_eq!("world", huff_decode(&res[7..11]));

    // Next is not indexed
    assert_eq!(&[15, 47, 0x80 | 3], &res[11..14]);
    assert_eq!("zomg", huff_decode(&res[14..]));
}

#[test]
fn test_large_size_update() {
    let mut encoder = Encoder::default();

    encoder.update_max_size(1912930560);
    assert_eq!(Some(SizeUpdate::One(1912930560)), encoder.size_update);

    let mut dst = BytesMut::with_capacity(6);
    encoder.encode_size_updates(&mut dst);
    assert_eq!([63, 225, 129, 148, 144, 7], &dst[..]);
}

#[test]
#[ignore]
fn test_evicted_overflow() {
    // Not sure what the best way to do this is.
}

fn encode(e: &mut Encoder, hdrs: Vec<Header<Option<HeaderName>>>) -> BytesMut {
    let mut dst = BytesMut::with_capacity(1024);
    e.encode(hdrs, &mut dst);
    dst
}

fn method(s: &str) -> Header<Option<HeaderName>> {
    Header::Method(Method::from_bytes(s.as_bytes()).unwrap())
}

fn header(name: &str, val: &str) -> Header<Option<HeaderName>> {
    let name = HeaderName::from_bytes(name.as_bytes()).unwrap();
    let value = HeaderValue::from_bytes(val.as_bytes()).unwrap();

    Header::Field {
        name: Some(name),
        value,
    }
}

fn huff_decode(src: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    huffman::decode(src, &mut buf).unwrap()
}
