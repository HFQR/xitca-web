use core::iter::FromIterator;

use crate::{
    h2::proto::{
        frame,
        hpack::{Encoder, huffman},
    },
    http::HeaderValue,
};

use super::*;

#[test]
fn test_nameless_header_at_resume() {
    let mut encoder = Encoder::default();
    let mut dst = BytesMut::new();

    let headers = Headers::new(
        StreamId::ZERO,
        (),
        HeaderMap::from_iter(vec![
            (HeaderName::from_static("hello"), HeaderValue::from_static("world")),
            (HeaderName::from_static("hello"), HeaderValue::from_static("zomg")),
            (HeaderName::from_static("hello"), HeaderValue::from_static("sup")),
        ]),
    );

    let continuation = headers
        .encode(&mut encoder, &mut (&mut dst).limit(frame::HEADER_LEN + 8))
        .unwrap();

    assert_eq!(17, dst.len());
    assert_eq!([0, 0, 8, 1, 0, 0, 0, 0, 0], &dst[0..9]);
    assert_eq!(&[0x40, 0x80 | 4], &dst[9..11]);
    assert_eq!("hello", huff_decode(&dst[11..15]));
    assert_eq!(0x80 | 4, dst[15]);

    let mut world = dst[16..17].to_owned();

    dst.clear();

    assert!(
        continuation
            .encode(&mut (&mut dst).limit(frame::HEADER_LEN + 16))
            .is_none()
    );

    world.extend_from_slice(&dst[9..12]);
    assert_eq!("world", huff_decode(&world));

    assert_eq!(24, dst.len());
    assert_eq!([0, 0, 15, 9, 4, 0, 0, 0, 0], &dst[0..9]);

    // // Next is not indexed
    assert_eq!(&[15, 47, 0x80 | 3], &dst[12..15]);
    assert_eq!("zomg", huff_decode(&dst[15..18]));
    assert_eq!(&[15, 47, 0x80 | 3], &dst[18..21]);
    assert_eq!("sup", huff_decode(&dst[21..]));
}

fn huff_decode(src: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    huffman::decode(src, &mut buf).unwrap()
}

#[test]
fn test_connect_request_pseudo_headers_omits_path_and_scheme() {
    // CONNECT requests MUST NOT include :scheme & :path pseudo-header fields
    // See: https://datatracker.ietf.org/doc/html/rfc9113#section-8.5

    assert_eq!(
        Pseudo::request(Method::CONNECT, Uri::from_static("https://example.com:8443"), None),
        Pseudo {
            method: Method::CONNECT.into(),
            authority: BytesStr::from_static("example.com:8443").into(),
            ..Default::default()
        }
    );

    assert_eq!(
        Pseudo::request(Method::CONNECT, Uri::from_static("https://example.com/test"), None),
        Pseudo {
            method: Method::CONNECT.into(),
            authority: BytesStr::from_static("example.com").into(),
            ..Default::default()
        }
    );

    assert_eq!(
        Pseudo::request(Method::CONNECT, Uri::from_static("example.com:8443"), None),
        Pseudo {
            method: Method::CONNECT.into(),
            authority: BytesStr::from_static("example.com:8443").into(),
            ..Default::default()
        }
    );
}

#[test]
fn test_extended_connect_request_pseudo_headers_includes_path_and_scheme() {
    // On requests that contain the :protocol pseudo-header field, the
    // :scheme and :path pseudo-header fields of the target URI (see
    // Section 5) MUST also be included.
    // See: https://datatracker.ietf.org/doc/html/rfc8441#section-4

    assert_eq!(
        Pseudo::request(
            Method::CONNECT,
            Uri::from_static("https://example.com:8443"),
            Some(BytesStr::from_static("the-bread-protocol"))
        ),
        Pseudo {
            method: Method::CONNECT.into(),
            authority: BytesStr::from_static("example.com:8443").into(),
            scheme: BytesStr::from_static("https").into(),
            path: BytesStr::from_static("/").into(),
            protocol: Some(BytesStr::from_static("the-bread-protocol")),
            ..Default::default()
        }
    );

    assert_eq!(
        Pseudo::request(
            Method::CONNECT,
            Uri::from_static("https://example.com:8443/test"),
            Some(BytesStr::from_static("the-bread-protocol"))
        ),
        Pseudo {
            method: Method::CONNECT.into(),
            authority: BytesStr::from_static("example.com:8443").into(),
            scheme: BytesStr::from_static("https").into(),
            path: BytesStr::from_static("/test").into(),
            protocol: Some(BytesStr::from_static("the-bread-protocol")),
            ..Default::default()
        }
    );

    assert_eq!(
        Pseudo::request(
            Method::CONNECT,
            Uri::from_static("http://example.com/a/b/c"),
            Some(BytesStr::from_static("the-bread-protocol"))
        ),
        Pseudo {
            method: Method::CONNECT.into(),
            authority: BytesStr::from_static("example.com").into(),
            scheme: BytesStr::from_static("http").into(),
            path: BytesStr::from_static("/a/b/c").into(),
            protocol: Some(BytesStr::from_static("the-bread-protocol")),
            ..Default::default()
        }
    );
}

#[test]
fn test_options_request_with_empty_path_has_asterisk_as_pseudo_path() {
    // an OPTIONS request for an "http" or "https" URI that does not include a path component;
    // these MUST include a ":path" pseudo-header field with a value of '*' (see Section 7.1 of [HTTP]).
    // See: https://datatracker.ietf.org/doc/html/rfc9113#section-8.3.1
    assert_eq!(
        Pseudo::request(Method::OPTIONS, Uri::from_static("example.com:8080"), None,),
        Pseudo {
            method: Method::OPTIONS.into(),
            authority: BytesStr::from_static("example.com:8080").into(),
            path: BytesStr::from_static("*").into(),
            ..Default::default()
        }
    );
}

#[test]
fn test_non_option_and_non_connect_requests_include_path_and_scheme() {
    let methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::HEAD,
        Method::PATCH,
        Method::TRACE,
    ];

    for method in methods {
        assert_eq!(
            Pseudo::request(method.clone(), Uri::from_static("http://example.com:8080"), None,),
            Pseudo {
                method: method.clone().into(),
                authority: BytesStr::from_static("example.com:8080").into(),
                scheme: BytesStr::from_static("http").into(),
                path: BytesStr::from_static("/").into(),
                ..Default::default()
            }
        );
        assert_eq!(
            Pseudo::request(method.clone(), Uri::from_static("https://example.com/a/b/c"), None,),
            Pseudo {
                method: method.into(),
                authority: BytesStr::from_static("example.com").into(),
                scheme: BytesStr::from_static("https").into(),
                path: BytesStr::from_static("/a/b/c").into(),
                ..Default::default()
            }
        );
    }
}
