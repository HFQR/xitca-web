use core::{future::poll_fn, net::SocketAddr, pin::pin};

use crate::{
    body::{BodyError, BodySize, ResponseBody},
    bytes::{Buf, Bytes},
    date::DateTimeHandle,
    h3::{Connection, Error},
    http::{
        header::{HeaderValue, CONTENT_LENGTH, DATE, HOST},
        Method, Request, Response,
    },
};
use ::h3_quinn::quinn::Endpoint;
use futures_core::stream::Stream;
use xitca_http::date::DateTime;

pub(crate) async fn send<B, E>(
    stream: &mut Connection,
    date: DateTimeHandle<'_>,
    req: Request<B>,
) -> Result<Response<ResponseBody>, Error>
where
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    let (parts, body) = req.into_parts();
    let mut req = Request::from_parts(parts, ());

    // Content length and is body is in eof state.
    let is_eof = match BodySize::from_stream(&body) {
        BodySize::None => {
            req.headers_mut().remove(CONTENT_LENGTH);
            true
        }
        BodySize::Stream => {
            req.headers_mut().remove(CONTENT_LENGTH);
            false
        }
        BodySize::Sized(0) => {
            req.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
            true
        }
        BodySize::Sized(len) => {
            let mut buf = itoa::Buffer::new();
            req.headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from_str(buf.format(len)).unwrap());
            false
        }
    };

    // remove host header if present, some web server may send 400 bad request if host header is present.
    req.headers_mut().remove(HOST);

    if !req.headers().contains_key(DATE) {
        let date = date.with_date(HeaderValue::from_bytes).unwrap();
        req.headers_mut().append(DATE, date);
    }

    let is_head_method = *req.method() == Method::HEAD;

    let mut stream = stream.send_request(req).await?;

    if !is_eof {
        let mut body = pin!(body);
        while let Some(bytes) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
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

pub(crate) async fn connect(
    endpoint: &Endpoint,
    addrs: impl Iterator<Item = SocketAddr>,
    hostname: &str,
) -> Result<Connection, Error> {
    let mut err = None;
    for addr in addrs {
        match _connect(endpoint, addr, hostname).await {
            Ok(connection) => return Ok(connection),
            Err(e) => err = Some(e),
        }
    }
    Err(err.unwrap())
}

async fn _connect(client: &Endpoint, addr: SocketAddr, hostname: &str) -> Result<Connection, Error> {
    let conn = client.connect(addr, hostname)?.await?;

    let (mut task, conn) = h3::client::new(h3_quinn::Connection::new(conn)).await?;

    tokio::spawn(async move {
        let _ = poll_fn(|cx| task.poll_close(cx)).await;
    });

    Ok(conn)
}
