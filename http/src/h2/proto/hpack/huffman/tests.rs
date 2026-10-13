use super::*;

fn decode(src: &[u8]) -> Result<BytesMut, DecoderError> {
    let mut buf = BytesMut::new();
    super::decode(src, &mut buf)
}

#[test]
fn decode_single_byte() {
    assert_eq!("o", decode(&[0b00111111]).unwrap());
    assert_eq!("0", decode(&[7]).unwrap());
    assert_eq!("A", decode(&[(0x21 << 2) + 3]).unwrap());
}

#[test]
fn single_char_multi_byte() {
    assert_eq!("#", decode(&[255, 160 + 15]).unwrap());
    assert_eq!("$", decode(&[255, 200 + 7]).unwrap());
    assert_eq!("\x0a", decode(&[255, 255, 255, 240 + 3]).unwrap());
}

#[test]
fn multi_char() {
    assert_eq!("!0", decode(&[254, 1]).unwrap());
    assert_eq!(" !", decode(&[0b01010011, 0b11111000]).unwrap());
}

#[test]
fn encode_single_byte() {
    let mut dst = BytesMut::with_capacity(1);

    encode(b"o", &mut dst);
    assert_eq!(&dst[..], &[0b00111111]);

    dst.clear();
    encode(b"0", &mut dst);
    assert_eq!(&dst[..], &[7]);

    dst.clear();
    encode(b"A", &mut dst);
    assert_eq!(&dst[..], &[(0x21 << 2) + 3]);
}

#[test]
fn encode_decode_str() {
    const DATA: &[&str] = &[
        "hello world",
        ":method",
        ":scheme",
        ":authority",
        "yahoo.co.jp",
        "GET",
        "http",
        ":path",
        "/images/top/sp2/cmn/logo-ns-130528.png",
        "example.com",
        "hpack-test",
        "xxxxxxx1",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.8; rv:16.0) Gecko/20100101 Firefox/16.0",
        "accept",
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        "cookie",
        "B=76j09a189a6h4&b=3&s=0b",
        "TE",
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Morbi non bibendum libero. \
         Etiam ultrices lorem ut.",
    ];

    for s in DATA {
        let mut dst = BytesMut::with_capacity(s.len());

        encode(s.as_bytes(), &mut dst);

        let decoded = decode(&dst).unwrap();

        assert_eq!(&decoded[..], s.as_bytes());
    }
}

#[test]
fn encode_decode_u8() {
    const DATA: &[&[u8]] = &[b"\0", b"\0\0\0", b"\0\x01\x02\x03\x04\x05", b"\xFF\xF8"];

    for s in DATA {
        let mut dst = BytesMut::with_capacity(s.len());

        encode(s, &mut dst);

        let decoded = decode(&dst).unwrap();

        assert_eq!(&decoded[..], &s[..]);
    }
}
