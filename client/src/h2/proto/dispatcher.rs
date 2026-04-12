use core::{cmp, future::poll_fn, pin::pin};

use std::io;

use ::h2::client;
use xitca_http::{date::DateTime, http::header::CONTENT_TYPE};
use xitca_io::io::{AsyncIo, PollIoAdapter};

use crate::{
    body::{Body, BodyError, Frame, ResponseBody, SizeHint},
    bytes::Bytes,
    date::DateTimeHandle,
    h2::{Connection, Error, body::ResponseBody as H2ResponseBody},
    http::{
        self,
        const_header_value::GRPC,
        header::{CONNECTION, CONTENT_LENGTH, DATE, HOST, HeaderValue, TRANSFER_ENCODING, UPGRADE},
        method::Method,
    },
};

pub(crate) async fn send<B>(
    stream: &mut Connection,
    date: DateTimeHandle<'_>,
    req: http::Request<B>,
) -> Result<http::Response<ResponseBody>, Error>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Send + Sync + 'static,
    BodyError: From<B::Error>,
{
    let (parts, body) = req.into_parts();
    let mut req = http::Request::from_parts(parts, ());

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
        SizeHint::Exact(len) => {
            let mut buf = itoa::Buffer::new();
            req.headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from_str(buf.format(len)).unwrap());
            false
        }
    };

    // TODO: consider skipping other headers according to:
    //       https://tools.ietf.org/html/rfc7540#section-8.1.2.2
    // omit HTTP/1.x only headers
    req.headers_mut().remove(CONNECTION);
    req.headers_mut().remove(TRANSFER_ENCODING);
    req.headers_mut().remove(UPGRADE);

    // remove host header if present, some web server may send 400 bad request if host header is present (like nginx)
    req.headers_mut().remove(HOST);

    if !req.headers().contains_key(DATE) {
        let date = date.with_date_header(Clone::clone);
        req.headers_mut().append(DATE, date);
    }

    let mut end_stream = is_eof;
    let mut is_head_method = false;

    match *req.method() {
        Method::CONNECT => {
            #[allow(unused_mut)]
            let mut protocol = ::h2::ext::Protocol::from_static("connect-ip");

            #[cfg(feature = "websocket")]
            {
                // a runtime behavior depend on http-ws crate to properly insert extension type into
                if let Some(p) = req.extensions_mut().remove::<http_ws::Http2WsProtocol>() {
                    protocol = ::h2::ext::Protocol::from(p.as_ref());
                }
            }

            req.extensions_mut().insert(protocol);

            // CONNECT establishes a tunnel — the stream must stay open for bidirectional data.
            end_stream = false;
        }
        Method::POST if req.headers().get(CONTENT_TYPE).is_some_and(|val| val == GRPC) && is_eof => {
            // grpc stream may start with zero body. in that case
            end_stream = false;
        }
        Method::HEAD => is_head_method = true,
        _ => {}
    }

    let (fut, mut stream) = stream.send_request(req, end_stream)?;

    if !is_eof {
        let mut body = pin!(body);

        'out: loop {
            match poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
                Some(Ok(Frame::Data(mut chunk))) => {
                    while !chunk.is_empty() {
                        let len = chunk.len();

                        stream.reserve_capacity(len);

                        let cap = poll_fn(|cx| stream.poll_capacity(cx))
                            .await
                            .ok_or_else(|| Error::Body(io::Error::from(io::ErrorKind::UnexpectedEof).into()))??;

                        // Split chuck to writeable size and send to client.
                        let bytes = chunk.split_to(cmp::min(cap, len));

                        let end_stream = chunk.is_empty() && body.is_end_stream();

                        stream.send_data(bytes, end_stream)?;

                        if end_stream {
                            break 'out;
                        }
                    }
                }
                Some(Ok(Frame::Trailers(trailers))) => {
                    stream.send_trailers(trailers)?;
                    break;
                }
                None => {
                    stream.send_data(Bytes::new(), true)?;
                    break;
                }
                Some(Err(e)) => return Err(BodyError::from(e).into()),
            }
        }
    }

    let res = fut.await?;

    let res = if is_head_method {
        res.map(|_| ResponseBody::Eof)
    } else {
        res.map(|body| ResponseBody::H2(H2ResponseBody::new(stream, body)))
    };

    Ok(res)
}

pub(crate) async fn handshake<S>(stream: S) -> Result<Connection, Error>
where
    S: AsyncIo + Send + 'static,
{
    let (conn, task) = client::Builder::new()
        .enable_push(false)
        .handshake(PollIoAdapter(stream))
        .await?;

    tokio::spawn(async {
        let _ = task.await;
    });

    Ok(conn)
}
