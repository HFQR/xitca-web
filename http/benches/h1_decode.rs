use std::time::SystemTime;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use httpdate::HttpDate;
use tokio::time::Instant;
use xitca_http::{
    bytes::BytesMut,
    date::{DATE_VALUE_LENGTH, DateTime},
    h1::proto::context::Context,
};

struct DT([u8; DATE_VALUE_LENGTH]);

impl DT {
    fn dummy_date_time() -> Self {
        let mut date = [0; DATE_VALUE_LENGTH];
        date.copy_from_slice(HttpDate::from(SystemTime::now()).to_string().as_bytes());
        DT(date)
    }
}

impl DateTime for DT {
    const DATE_VALUE_LENGTH: usize = DATE_VALUE_LENGTH;

    fn with_date<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&[u8]) -> O,
    {
        f(&self.0)
    }

    fn now(&self) -> Instant {
        todo!()
    }
}

fn decode(c: &mut Criterion) {
    let dt = DT::dummy_date_time();

    let mut ctx = Context::<_, 8>::new(&dt, false);

    let req = b"\
    GET /HFQR/xitca-web HTTP/1.1\r\n\
    Host: server\r\n\
    User-Agent: Mozilla/5.0 (Windows NT 6.1; Win64; x64; rv:47.0) Gecko/20100101 Firefox/47.0\r\n\
    Cookie: uid=12345678901234567890\r\n\
    Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\r\n\
    Accept-Language: en-US,en;q=0.5\r\n\
    Connection: keep-alive\r\n\
    \r\n\
    ";

    let buf = BytesMut::from(&req[..]);

    c.bench_function("h1_decode", |b| {
        b.iter(|| {
            let (req, _) = ctx
                .decode_head::<{ usize::MAX }>(black_box(&mut buf.clone()))
                .unwrap()
                .unwrap();
            let mut headers = req.into_parts().0.headers;
            headers.clear();
            ctx.replace_headers(headers);
        });
    });
}

criterion_group!(benches, decode);
criterion_main!(benches);
