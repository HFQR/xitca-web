use super::*;
use assert_matches::assert_matches;

#[test]
fn request_has_no_authority_nor_host() {
    //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
    //= type=test
    //# If the :scheme pseudo-header field identifies a scheme that has a
    //# mandatory authority component (including "http" and "https"), the
    //# request MUST contain either an :authority pseudo-header field or a
    //# Host header field.
    let headers = Header::try_from(vec![
        (b":method", Method::GET.as_str()).into(),
        (b":scheme", b"https").into(),
        (b":path", b"/").into(),
    ])
    .unwrap();
    assert!(headers.pseudo.authority.is_none());
    assert_matches!(headers.into_request_parts(), Err(HeaderError::MissingAuthority));
}

#[test]
fn request_has_empty_authority() {
    //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
    //= type=test
    //# If these fields are present, they MUST NOT be
    //# empty.
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":authority", b"").into(),
        ]),
        Err(HeaderError::InvalidHeaderValue(_))
    );
}

#[test]
fn request_has_empty_host() {
    //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
    //= type=test
    //# If these fields are present, they MUST NOT be
    //# empty.
    let headers = Header::try_from(vec![
        (b":method", Method::GET.as_str()).into(),
        (b":scheme", b"https").into(),
        (b":path", b"/").into(),
        (b"host", b"").into(),
    ])
    .unwrap();
    assert_matches!(headers.into_request_parts(), Err(HeaderError::InvalidRequest(_)));
}

#[test]
fn request_has_authority() {
    //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
    //= type=test
    //# If the :scheme pseudo-header field identifies a scheme that has a
    //# mandatory authority component (including "http" and "https"), the
    //# request MUST contain either an :authority pseudo-header field or a
    //# Host header field.
    let headers = Header::try_from(vec![
        (b":method", Method::GET.as_str()).into(),
        (b":scheme", b"https").into(),
        (b":authority", b"test.com").into(),
        (b":path", b"/").into(),
    ])
    .unwrap();
    assert_matches!(headers.into_request_parts(), Ok(_));
}

#[test]
fn request_has_host() {
    //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
    //= type=test
    //# If the :scheme pseudo-header field identifies a scheme that has a
    //# mandatory authority component (including "http" and "https"), the
    //# request MUST contain either an :authority pseudo-header field or a
    //# Host header field.
    let headers = Header::try_from(vec![
        (b":method", Method::GET.as_str()).into(),
        (b":scheme", b"https").into(),
        (b":path", b"/").into(),
        (b"host", b"test.com").into(),
    ])
    .unwrap();
    assert!(headers.pseudo.authority.is_none());
    assert_matches!(headers.into_request_parts(), Ok(_));
}

#[test]
fn request_has_same_host_and_authority() {
    //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
    //= type=test
    //# If both fields are present, they MUST contain the same value.
    let headers = Header::try_from(vec![
        (b":method", Method::GET.as_str()).into(),
        (b":scheme", b"https").into(),
        (b":authority", b"test.com").into(),
        (b":path", b"/").into(),
        (b"host", b"test.com").into(),
    ])
    .unwrap();
    assert_matches!(headers.into_request_parts(), Ok(_));
}
#[test]
fn request_has_different_host_and_authority() {
    //= https://www.rfc-editor.org/rfc/rfc9114#section-4.3.1
    //= type=test
    //# If both fields are present, they MUST contain the same value.
    let headers = Header::try_from(vec![
        (b":method", Method::GET.as_str()).into(),
        (b":scheme", b"https").into(),
        (b":authority", b"authority.com").into(),
        (b":path", b"/").into(),
        (b"host", b"host.com").into(),
    ])
    .unwrap();
    assert_matches!(headers.into_request_parts(), Err(HeaderError::ContradictedAuthority));
}

#[test]
fn rejects_connection_specific_header() {
    for name in [
        b"connection".as_slice(),
        b"keep-alive",
        b"proxy-connection",
        b"transfer-encoding",
        b"upgrade",
    ] {
        assert_matches!(
            Header::try_from(vec![
                (b":method", Method::GET.as_str()).into(),
                (b":authority", b"test.com").into(),
                (name, b"close").into(),
            ]),
            Err(HeaderError::ConnectionSpecific(_))
        );
    }
}

#[test]
fn accepts_te_trailers_only() {
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":authority", b"test.com").into(),
            (b"te", b"trailers").into(),
        ]),
        Ok(_)
    );
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":authority", b"test.com").into(),
            (b"te", b"gzip").into(),
        ]),
        Err(HeaderError::InvalidTeValue)
    );
}

#[test]
fn rejects_duplicate_pseudo() {
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":method", Method::POST.as_str()).into(),
            (b":authority", b"test.com").into(),
        ]),
        Err(HeaderError::DuplicatePseudo(":method"))
    );
}

#[test]
fn regular_request_requires_scheme_and_path() {
    // Missing :scheme.
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":authority", b"test.com").into(),
            (b":path", b"/").into(),
        ])
        .unwrap()
        .into_request_parts(),
        Err(HeaderError::MissingScheme)
    );
    // Missing :path.
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b":scheme", b"https").into(),
            (b":authority", b"test.com").into(),
        ])
        .unwrap()
        .into_request_parts(),
        Err(HeaderError::MissingPath)
    );
}

#[test]
fn strict_connect_rejects_scheme_or_path() {
    // CONNECT with :scheme.
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::CONNECT.as_str()).into(),
            (b":scheme", b"https").into(),
            (b":authority", b"test.com:443").into(),
        ])
        .unwrap()
        .into_request_parts(),
        Err(HeaderError::ConnectWithScheme)
    );
    // CONNECT with :path.
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::CONNECT.as_str()).into(),
            (b":authority", b"test.com:443").into(),
            (b":path", b"/").into(),
        ])
        .unwrap()
        .into_request_parts(),
        Err(HeaderError::ConnectWithPath)
    );
}

#[test]
fn strict_connect_accepts_authority_only() {
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::CONNECT.as_str()).into(),
            (b":authority", b"test.com:443").into(),
        ])
        .unwrap()
        .into_request_parts(),
        Ok(_)
    );
}

#[test]
fn rejects_pseudo_after_regular() {
    assert_matches!(
        Header::try_from(vec![
            (b":method", Method::GET.as_str()).into(),
            (b"x-foo", b"bar").into(),
            (b":authority", b"test.com").into(),
        ]),
        Err(HeaderError::PseudoAfterRegular)
    );
}

#[test]
fn preserves_duplicate_headers() {
    let headers = Header::try_from(vec![
        (b":method", Method::GET.as_str()).into(),
        (b":authority", b"test.com").into(),
        (b"set-cookie", b"foo=foo").into(),
        (b"set-cookie", b"bar=bar").into(),
        (b"other-header", b"other-header-value").into(),
    ])
    .unwrap();

    assert_eq!(
        headers
            .clone()
            .into_iter()
            .filter(|h| h.name.as_ref() == b"set-cookie")
            .collect::<Vec<_>>(),
        vec![
            HeaderField {
                name: std::borrow::Cow::Borrowed(b"set-cookie"),
                value: std::borrow::Cow::Borrowed(b"foo=foo")
            },
            HeaderField {
                name: std::borrow::Cow::Borrowed(b"set-cookie"),
                value: std::borrow::Cow::Borrowed(b"bar=bar")
            }
        ]
    );
    assert_eq!(
        headers
            .into_iter()
            .filter(|h| h.name.as_ref() == b"other-header")
            .collect::<Vec<_>>(),
        vec![HeaderField {
            name: std::borrow::Cow::Borrowed(b"other-header"),
            value: std::borrow::Cow::Borrowed(b"other-header-value")
        },]
    );
}
