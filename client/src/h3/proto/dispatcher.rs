use std::net::SocketAddr;

use futures_core::stream::Stream;
use futures_util::future::poll_fn;
use h3_quinn::quinn::Endpoint;
use xitca_http::{
    bytes::{Buf, Bytes},
    date::DateTime,
    error::BodyError,
    http::{
        header::{HeaderValue, CONTENT_LENGTH, DATE},
        Method, Request, Response, Version,
    },
};

use crate::{
    body::{RequestBody, RequestBodySize, ResponseBody},
    date::DateTimeHandle,
    h3::{Connection, Error},
};

pub(crate) async fn send<B, E>(
    stream: &mut Connection,
    date: DateTimeHandle<'_>,
    mut req: Request<RequestBody<B>>,
) -> Result<Response<ResponseBody<'static>>, Error>
where
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    *req.version_mut() = Version::HTTP_3;

    let (parts, body) = req.into_parts();
    let mut req = Request::from_parts(parts, ());

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

    if !req.headers().contains_key(DATE) {
        let date = date.with_date(HeaderValue::from_bytes).unwrap();
        req.headers_mut().append(DATE, date);
    }

    let is_eof = body.is_eof();
    let is_head_method = *req.method() == Method::HEAD;

    let mut stream = stream.send_request(req).await?;

    if !is_eof {
        tokio::pin!(body);
        while let Some(bytes) = body.as_mut().next().await {
            let bytes = bytes.map_err(BodyError::from)?;
            stream.send_data(bytes).await?;
        }
    }

    stream.finish().await?;

    let res = stream.recv_response().await?;

    let res = if is_head_method {
        res.map(|_| ResponseBody::Eof)
    } else {
        let body = async_stream::stream! {
            while let Some(bytes) = stream.recv_data().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)? {
                yield Ok(Bytes::copy_from_slice(bytes.chunk()));
            }
        };
        let body = crate::h3::body::ResponseBody(Box::pin(body));

        res.map(|_| ResponseBody::H3(body))
    };

    Ok(res)
}

pub(crate) async fn connect(client: &Endpoint, addr: &SocketAddr, hostname: &str) -> Result<Connection, Error> {
    let conn = client.connect(*addr, hostname)?.await?;

    let (mut task, conn) = h3::client::new(h3_quinn::Connection::new(conn)).await?;

    tokio::spawn(async move {
        poll_fn(|cx| task.poll_close(cx))
            .await
            .expect("http3 connection failed");
    });

    Ok(conn)
}
