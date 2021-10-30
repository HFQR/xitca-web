use std::cmp;

use futures_core::stream::Stream;
use futures_util::future::poll_fn;
use h2::client::{self, Connection, SendRequest};
use h2::RecvStream;
use tokio::io::{AsyncRead, AsyncWrite};
use xitca_http::{
    bytes::Bytes,
    date::DateTime,
    error::BodyError,
    http::{
        self,
        header::{HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING},
        method::Method,
        version::Version,
    },
};

use crate::{
    body::{RequestBody, RequestBodySize},
    date::DateTimeHandle,
    h2::error::Error,
};

pub(crate) async fn send<B, E>(
    stream: &mut SendRequest<Bytes>,
    date: DateTimeHandle<'_>,
    mut req: http::Request<RequestBody<B>>,
) -> Result<(http::Response<RecvStream>, bool), Error>
where
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    *req.version_mut() = Version::HTTP_2;

    let (parts, body) = req.into_parts();
    let mut req = http::Request::from_parts(parts, ());

    // Content length
    match body.size() {
        RequestBodySize::None | RequestBodySize::Stream => {
            req.headers_mut().remove(CONTENT_LENGTH);
        }
        RequestBodySize::Sized(len) if len == 0 => {
            req.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
        }
        RequestBodySize::Sized(len) => {
            let mut buf = itoa::Buffer::new();
            req.headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from_str(buf.format(len)).unwrap());
        }
    };

    // TODO: consider skipping other headers according to:
    //       https://tools.ietf.org/html/rfc7540#section-8.1.2.2
    // omit HTTP/1.x only headers
    req.headers_mut().remove(CONNECTION);
    req.headers_mut().remove(TRANSFER_ENCODING);

    if !req.headers().contains_key(DATE) {
        let date = date.with_date(HeaderValue::from_bytes).unwrap();
        req.headers_mut().append(DATE, date);
    }

    let is_eof = body.is_eof();
    let is_head_method = *req.method() == Method::HEAD;

    let (fut, mut tx) = stream.send_request(req, is_eof)?;

    if !is_eof {
        tokio::pin!(body);

        while let Some(bytes) = body.as_mut().next().await {
            let mut bytes = bytes?;

            while bytes.len() > 0 {
                tx.reserve_capacity(bytes.len());

                let size = poll_fn(|cx| tx.poll_capacity(cx)).await.unwrap()?;

                let offset = cmp::min(size, bytes.len());
                let buf = bytes.split_to(offset);

                tx.send_data(buf, false)?;
            }
        }

        tx.send_data(Bytes::new(), true)?;
        tx.reserve_capacity(0);
    }

    let res = fut.await?;
    Ok((res, is_head_method))
}

pub(crate) async fn handshake<S>(stream: S) -> Result<(SendRequest<Bytes>, Connection<S>), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    client::handshake(stream).await.map_err(Into::into)
}
