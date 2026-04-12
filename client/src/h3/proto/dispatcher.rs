use core::{future::poll_fn, net::SocketAddr, pin::pin};

use ::h3_quinn::quinn::Endpoint;
use xitca_http::date::DateTime;

use crate::{
    body::{Body, BodyError, BodyExt, Frame, ResponseBody, SizeHint},
    bytes::Bytes,
    date::DateTimeHandle,
    h3::{Connection, Error},
    http::{
        Method, Request, Response,
        header::{CONTENT_LENGTH, DATE, HOST, HeaderValue},
    },
};

use super::super::body::ResponseBody as H3ResponseBody;

pub(crate) async fn send<B>(
    stream: &mut Connection,
    date: DateTimeHandle<'_>,
    req: Request<B>,
) -> Result<Response<ResponseBody>, Error>
where
    B: Body<Data = Bytes>,
    BodyError: From<B::Error>,
{
    let (parts, body) = req.into_parts();
    let mut req = Request::from_parts(parts, ());

    // Content length and is body is in eof state.
    let is_eof = match body.size_hint() {
        SizeHint::None => {
            req.headers_mut().remove(CONTENT_LENGTH);
            true
        }
        SizeHint::Unknown => {
            req.headers_mut().remove(CONTENT_LENGTH);
            false
        }
        SizeHint::Exact(0) => {
            req.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
            true
        }
        SizeHint::Exact(len) => {
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
        while let Some(frame) = body.as_mut().frame().await {
            let frame = frame.map_err(BodyError::from)?;
            match frame {
                Frame::Data(bytes) => stream.send_data(bytes).await?,
                Frame::Trailers(trailers) => {
                    stream.send_trailers(trailers).await?;
                    break;
                }
            }
        }
    }

    stream.finish().await?;

    let res = stream.recv_response().await?;

    let res = if is_head_method {
        res.map(|_| ResponseBody::Eof)
    } else {
        res.map(|_| ResponseBody::H3(H3ResponseBody::new(stream)))
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
