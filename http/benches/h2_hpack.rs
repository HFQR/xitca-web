use std::{hint::black_box, io::Cursor};

use criterion::{Criterion, criterion_group, criterion_main};
use xitca_http::{
    bytes::BytesMut,
    h2::__private_bench::{Decoder, Encoder, Header},
    http::{HeaderName, HeaderValue, Method, StatusCode},
};
use xitca_unsafe_collection::bytes::BytesStr;

const TYPICAL_RESPONSE: [Header<Option<HeaderName>>; 8] = [
    Header::Status(StatusCode::OK),
    Header::Field {
        name: Some(HeaderName::from_static("content-type")),
        value: HeaderValue::from_static("text/html; charset=utf-8"),
    },
    Header::Field {
        name: Some(HeaderName::from_static("content-length")),
        value: HeaderValue::from_static("1024"),
    },
    Header::Field {
        name: Some(HeaderName::from_static("date")),
        value: HeaderValue::from_static("Wed, 29 Apr 2026 12:34:56 GMT"),
    },
    Header::Field {
        name: Some(HeaderName::from_static("server")),
        value: HeaderValue::from_static("xitca-web/0.9"),
    },
    Header::Field {
        name: Some(HeaderName::from_static("cache-control")),
        value: HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    },
    Header::Field {
        name: Some(HeaderName::from_static("vary")),
        value: HeaderValue::from_static("Accept-Encoding, Origin"),
    },
    Header::Field {
        name: Some(HeaderName::from_static("x-request-id")),
        value: HeaderValue::from_static("01HXY3Z7QABCDEF1234567890ABCD"),
    },
];

const TYPICAL_REQUEST: [Header<Option<HeaderName>>; 9] = [
    Header::Method(Method::GET),
    Header::Scheme(BytesStr::from_static("https")),
    Header::Authority(BytesStr::from_static("example.com")),
    Header::Path(BytesStr::from_static("/api/v1/items?limit=20")),
    Header::Field {
        name: Some(HeaderName::from_static("user-agent")),
        value: HeaderValue::from_static(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36",
        ),
    },
    Header::Field {
        name: Some(HeaderName::from_static("accept")),
        value: HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"),
    },
    Header::Field {
        name: Some(HeaderName::from_static("accept-language")),
        value: HeaderValue::from_static("en-US,en;q=0.9"),
    },
    Header::Field {
        name: Some(HeaderName::from_static("accept-encoding")),
        value: HeaderValue::from_static("gzip, deflate, br"),
    },
    Header::Field {
        name: Some(HeaderName::from_static("cookie")),
        value: HeaderValue::from_static("session=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NSJ9.abcdef"),
    },
];

fn long_value_header() -> [Header<Option<HeaderName>>; 1] {
    // Forces huff_len > 127, which used to trigger the byte-shift path.
    let value = "a".repeat(512);
    [Header::Field {
        name: Some(HeaderName::from_static("x-long")),
        value: HeaderValue::from_str(&value).unwrap(),
    }]
}

fn encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("h2_hpack_encode");

    group.bench_function("typical_response_cold", |b| {
        let mut dst = BytesMut::with_capacity(256);
        b.iter(|| {
            dst.clear();
            let mut encoder = Encoder::default();
            encoder.encode(black_box(TYPICAL_RESPONSE), &mut dst);
        });
    });

    group.bench_function("typical_response_warm", |b| {
        let mut encoder = Encoder::default();
        let mut dst = BytesMut::with_capacity(256);
        // Prime the dynamic table.
        encoder.encode(TYPICAL_RESPONSE, &mut dst);
        b.iter(move || {
            dst.clear();
            encoder.encode(black_box(TYPICAL_RESPONSE), &mut dst);
        });
    });

    group.bench_function("typical_request_cold", |b| {
        let mut dst = BytesMut::with_capacity(512);
        b.iter(move || {
            dst.clear();
            let mut encoder = Encoder::default();
            encoder.encode(black_box(TYPICAL_REQUEST), &mut dst);
        });
    });

    group.bench_function("long_value_512b", |b| {
        let headers = long_value_header();
        let mut dst = BytesMut::with_capacity(1024);
        b.iter(move || {
            dst.clear();
            let mut encoder = Encoder::default();
            encoder.encode(black_box(headers.clone()), &mut dst);
        });
    });

    group.finish();
}

fn decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("h2_hpack_decode");

    let encoded_response = {
        let mut encoder = Encoder::default();
        let mut dst = BytesMut::with_capacity(256);
        encoder.encode(TYPICAL_RESPONSE, &mut dst);
        dst
    };

    let encoded_request = {
        let mut encoder = Encoder::default();
        let mut dst = BytesMut::with_capacity(512);
        encoder.encode(TYPICAL_REQUEST, &mut dst);
        dst
    };

    group.bench_function("typical_response", |b| {
        b.iter(|| {
            let mut decoder = Decoder::default();
            let mut buf = encoded_response.clone();
            decoder
                .decode(&mut Cursor::new(black_box(&mut buf)), |h| {
                    black_box(h);
                })
                .unwrap();
        });
    });

    group.bench_function("typical_request", |b| {
        b.iter(|| {
            let mut decoder = Decoder::default();
            let mut buf = encoded_request.clone();
            decoder
                .decode(&mut Cursor::new(black_box(&mut buf)), |h| {
                    black_box(h);
                })
                .unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, encode, decode);
criterion_main!(benches);
