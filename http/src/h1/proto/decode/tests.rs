use super::*;

#[test]
fn connection_multiple_value() {
    let mut ctx = Context::<_, 4>::new(&());

    let head = b"\
            GET / HTTP/1.1\r\n\
            Connection: keep-alive, upgrade\r\n\
            \r\n\
            ";
    let mut buf = BytesMut::from(&head[..]);

    let _ = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
    assert!(!ctx.is_connection_closed());

    // this is a wrong connection header but instead of rejecting it the last close value is
    // used for the final value. there is no particular reason behind this behaviour and this
    // session of the test serves only as a consistency check to prevent regression.
    let head = b"\
            GET / HTTP/1.1\r\n\
            Connection: keep-alive, close, upgrade\r\n\
            \r\n\
            ";
    let mut buf = BytesMut::from(&head[..]);

    let _ = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
    assert!(ctx.is_connection_closed());

    let head = b"\
            GET / HTTP/1.1\r\n\
            Connection: close, keep-alive, upgrade\r\n\
            \r\n\
            ";
    let mut buf = BytesMut::from(&head[..]);

    let _ = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
    assert!(!ctx.is_connection_closed());
}

#[test]
fn transfer_encoding() {
    let mut ctx = Context::<_, 4>::new(&());

    let head = b"\
            GET / HTTP/1.1\r\n\
            Transfer-Encoding: gzip\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            ";
    let mut buf = BytesMut::from(&head[..]);

    let (req, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
    let mut iter = req.headers().get_all(TRANSFER_ENCODING).into_iter();
    assert_eq!(iter.next().unwrap().to_str().unwrap(), "gzip");
    assert_eq!(iter.next().unwrap().to_str().unwrap(), "chunked");
    assert!(
        matches!(decoder, TransferCoding::DecodeChunked { .. }),
        "transfer coding is not decoded to chunked"
    );

    ctx.reset();

    let head = b"\
    GET / HTTP/1.1\r\n\
    Transfer-Encoding: chunked\r\n\
    Transfer-Encoding: gzip\r\n\
    \r\n\
    ";
    let mut buf = BytesMut::from(&head[..]);

    let (req, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
    let mut iter = req.headers().get_all(TRANSFER_ENCODING).into_iter();
    assert_eq!(iter.next().unwrap().to_str().unwrap(), "chunked");
    assert_eq!(iter.next().unwrap().to_str().unwrap(), "gzip");
    assert!(
        matches!(decoder, TransferCoding::DecodeChunked { .. }),
        "transfer coding is not decoded to chunked"
    );

    ctx.reset();

    let head = b"\
            GET / HTTP/1.1\r\n\
            Transfer-Encoding: gzip, chunked\r\n\
            \r\n\
            ";
    let mut buf = BytesMut::from(&head[..]);

    let (req, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
    assert_eq!(
        req.headers().get(TRANSFER_ENCODING).unwrap().to_str().unwrap(),
        "gzip, chunked"
    );
    assert!(
        matches!(decoder, TransferCoding::DecodeChunked { .. }),
        "transfer coding is not decoded to chunked"
    );

    ctx.reset();

    let head = b"\
            GET / HTTP/1.1\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            ";
    let mut buf = BytesMut::from(&head[..]);

    let (req, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
    assert_eq!(
        req.headers().get(TRANSFER_ENCODING).unwrap().to_str().unwrap(),
        "chunked"
    );
    assert!(
        matches!(decoder, TransferCoding::DecodeChunked { .. }),
        "transfer coding is not decoded to chunked"
    );

    let head = b"\
            GET / HTTP/1.1\r\n\
            Transfer-Encoding: identity\r\n\
            \r\n\
            ";
    let mut buf = BytesMut::from(&head[..]);
    let (_, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
    assert!(
        matches!(decoder, TransferCoding::Eof),
        "transfer coding is not decoded to eof"
    );

    let head = b"\
            GET / HTTP/1.1\r\n\
            Transfer-Encoding: chunked, gzip\r\n\
            \r\n\
            ";
    let mut buf = BytesMut::from(&head[..]);
    let (_, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
    assert!(
        matches!(decoder, TransferCoding::DecodeChunked { .. }),
        "transfer coding is not decoded to chunked"
    );
}
