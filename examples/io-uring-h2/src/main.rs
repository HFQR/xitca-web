//! A Http/2 server returns Hello World String as Response.
//!
//! *. use h2c prior knowledge as protocol.
//! *. io_uring is a linux OS feature.

use std::{
    convert::Infallible,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::stream::Stream;
use xitca_http::{
    HttpServiceBuilder,
    bytes::Bytes,
    h2::dispatcher_uring::{Frame, RequestBody},
    http::{Request, RequestExt, Response, const_header_value::TEXT_UTF8, header::CONTENT_TYPE},
};
use xitca_service::{ServiceExt, fn_service};

fn main() -> io::Result<()> {
    xitca_server::Builder::new()
        .bind(
            "http/2",
            "127.0.0.1:8080",
            fn_service(handler).enclosed(
                HttpServiceBuilder::h2().io_uring(), // specify io_uring flavor of http service.
            ),
        )?
        .build()
        .wait()
}

async fn handler(_: Request<RequestExt<RequestBody>>) -> Result<Response<Once>, Infallible> {
    Ok(Response::builder()
        .header(CONTENT_TYPE, TEXT_UTF8)
        .body(Once::new())
        .unwrap())
}

// async fn handler(_: Request<RequestExt<xitca_http::h2::RequestBody>>) -> Result<Response<xitca_http::body::Once<Bytes>>, Infallible> {
//     Ok(Response::builder()
//         .header(CONTENT_TYPE, TEXT_UTF8)
//         .body(xitca_http::body::Once::new(Bytes::from_static(b"Hello World!")))
//         .unwrap())
// }

struct Once(Option<Frame>);

impl Once {
    fn new() -> Self {
        Self(Some(Frame::Data(Bytes::from_static(b"Hello World!"))))
    }
}

impl Stream for Once {
    type Item = Result<Frame, Infallible>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().0.take().map(Ok))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = b"Hello World!".len();
        (len, Some(len))
    }
}
