use core::{cmp, future::poll_fn, mem, pin::pin};

use std::io;

use ::h2::client;
use xitca_http::date::DateTime;
use xitca_io::io::{AsyncIo, PollIoAdapter};

use crate::{
    body::{Body, BodyError, Frame, ResponseBody, SizeHint},
    bytes::Bytes,
    date::DateTimeHandle,
    h2::{Connection, Error, body::ResponseBody as H2ResponseBody},
    http::{
        self,
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

    // yielded by the match so it needs no `mut`, which it would never be given
    // without the grpc feature.
    let is_grpc_stream = match *req.method() {
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
            false
        }
        // **A gRPC POST with no body is a streaming call**, and cannot be
        // anything else: a unary call always carries a body -- the length
        // prefix alone is five bytes -- so there is no valid request this
        // catches by accident. That makes it a characterisation rather than a
        // heuristic, which is what it has to be to decide something as
        // load-bearing as whether the caller is handed a placeholder head.
        #[cfg(feature = "grpc")]
        Method::POST if crate::grpc::is_grpc_request(&req) && is_eof => {
            // grpc stream may start with no response header and body
            end_stream = false;
            true
        }
        Method::HEAD => {
            is_head_method = true;
            false
        }
        _ => false,
    };

    let (fut, mut stream) = stream.send_request(req, end_stream)?;

    if !is_eof {
        let mut body = pin!(body);

        'out: loop {
            match poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
                Some(Ok(Frame::Data(mut chunk))) => {
                    let end_stream = body.is_end_stream();

                    if chunk.is_empty() && !end_stream {
                        continue;
                    }

                    loop {
                        let len = chunk.len();

                        stream.reserve_capacity(len);

                        let cap = poll_fn(|cx| stream.poll_capacity(cx))
                            .await
                            .ok_or_else(|| Error::Body(io::Error::from(io::ErrorKind::UnexpectedEof).into()))??;

                        let aval = cmp::min(cap, len);

                        let full_comsumed = len == aval;

                        let bytes = if full_comsumed {
                            mem::take(&mut chunk)
                        } else {
                            chunk.split_to(aval)
                        };

                        let end_stream = full_comsumed && end_stream;

                        stream.send_data(bytes, end_stream)?;

                        if end_stream {
                            break 'out;
                        }

                        if full_comsumed {
                            continue 'out;
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

    // **The body is always built before the response head exists.** Resolving
    // that head is the body's own job (`ResponseBody::poll_head`), which leaves
    // exactly one way a head is obtained and makes "hand the stream over before
    // the server has answered" a property any caller can have rather than a
    // special case wired into this function.
    let mut body = H2ResponseBody::new(stream, fut);

    // The one caller that takes it up. A grpc stream may be the client's turn
    // to speak -- the spec allows it, and a server authenticating a handshake
    // *must* read before it can decide what to answer -- so waiting for a head
    // that is waiting on our first message deadlocks both sides.
    if is_grpc_stream {
        return Ok(http::Response::new(ResponseBody::H2(body)));
    }

    // Everyone else is handed a response, and a response has a head: `status()`
    // and `headers()` are read synchronously all over this crate and by every
    // middleware -- `Redirect` decides on the status alone -- so for anything
    // but a stream that asked for it, the head is awaited here as it always was.
    poll_fn(|cx| body.poll_head(cx)).await?;
    let head = body.take_head().expect("poll_head resolves the head or errors");

    Ok(http::Response::from_parts(
        head,
        if is_head_method {
            ResponseBody::Eof
        } else {
            ResponseBody::H2(body)
        },
    ))
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
