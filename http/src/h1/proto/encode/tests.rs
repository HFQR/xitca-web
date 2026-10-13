use crate::{
    body::{BoxBody, Full},
    date::SystemTimeDateTimeHandler,
    http::{HeaderValue, Response},
};

use super::*;

#[test]
fn append_header() {
    let mut ctx = Context::<_, 64>::new(&SystemTimeDateTimeHandler);

    let mut res = Response::new(BoxBody::new(Full::new(Bytes::new())));

    res.headers_mut()
        .insert(CONNECTION, HeaderValue::from_static("keep-alive"));
    res.headers_mut()
        .append(CONNECTION, HeaderValue::from_static("upgrade"));

    let (parts, body) = res.into_parts();

    let mut buf = BytesMut::new();
    ctx.encode_head(parts, &body, &mut buf).unwrap();

    let mut header = [httparse::EMPTY_HEADER; 8];
    let mut res = httparse::Response::new(&mut header);

    let httparse::Status::Complete(_) = res.parse(buf.as_ref()).unwrap() else {
        panic!("failed to parse response")
    };

    for h in header {
        if h.name == "connection" {
            assert_eq!(h.value, b"keep-alive, upgrade");
        }
    }
}

#[test]
fn multi_set_cookie() {
    let mut ctx = Context::<_, 64>::new(&SystemTimeDateTimeHandler);

    let mut res = Response::new(BoxBody::new(Full::new(Bytes::new())));

    res.headers_mut()
        .insert(SET_COOKIE, HeaderValue::from_static("foo=foo"));
    res.headers_mut()
        .append(SET_COOKIE, HeaderValue::from_static("bar=bar"));

    let (parts, body) = res.into_parts();

    let mut buf = BytesMut::new();
    ctx.encode_head(parts, &body, &mut buf).unwrap();

    let mut header = [httparse::EMPTY_HEADER; 8];
    let mut res = httparse::Response::new(&mut header);

    let httparse::Status::Complete(_) = res.parse(buf.as_ref()).unwrap() else {
        panic!("failed to parse response")
    };

    assert_eq!(header[0].name, "set-cookie");
    assert_eq!(header[0].value, b"foo=foo");
    assert_eq!(header[1].name, "set-cookie");
    assert_eq!(header[1].value, b"bar=bar");
}
