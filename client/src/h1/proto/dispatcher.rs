use core::{
    future::poll_fn,
    pin::{pin, Pin},
};

use std::io;

use futures_core::Stream;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use xitca_http::{bytes::Buf, h1::proto::codec::TransferCoding};

use crate::{
    body::BodyError,
    bytes::{Bytes, BytesMut},
    date::DateTimeHandle,
    h1::Error,
    http::{
        self,
        header::{HeaderValue, HOST},
        Method,
    },
};

use super::context::Context;

pub(crate) async fn send<S, B, E>(
    stream: &mut S,
    date: DateTimeHandle<'_>,
    mut req: http::Request<B>,
) -> Result<(http::Response<()>, BytesMut, TransferCoding, bool), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    let is_head_method = *req.method() == Method::HEAD;

    if !req.headers().contains_key(HOST) {
        if let Some(host) = req.uri().host() {
            let mut buf = BytesMut::with_capacity(host.len() + 5);

            buf.extend_from_slice(host.as_bytes());
            match req.uri().port_u16() {
                None | Some(80) | Some(443) => {}
                Some(port) => {
                    buf.extend_from_slice(b":");
                    buf.extend_from_slice(port.to_string().as_bytes());
                }
            };
            req.headers_mut()
                .insert(HOST, HeaderValue::from_maybe_shared(buf.freeze()).unwrap());
        }
    }

    let (parts, body) = req.into_parts();

    // TODO: make const generic params configurable.
    let mut ctx = Context::<128>::new(&date);
    let mut buf = BytesMut::new();

    // encode request head and return transfer encoding for request body
    let encoder = ctx.encode_head(&mut buf, parts, &body)?;

    write_all_buf(&mut *stream, &mut buf).await?;
    poll_fn(|cx| Pin::new(&mut *stream).poll_flush(cx)).await?;

    // TODO: concurrent read write is needed in case server decide to do two way
    // streaming with very large body surpass socket buffer size.
    // (In rare case the server could starting streaming back resposne without read all the request body)

    // try to send request body.
    // continue to read response no matter the outcome.
    if send_inner(stream, encoder, body, &mut buf).await.is_err() {
        // an error indicate connection should be closed.
        ctx.set_close();
        // clear the buffer as there could be unfinished request data inside.
        buf.clear();
    }

    // read response head and get body decoder.
    loop {
        let n = stream.read_buf(&mut buf).await?;

        if n == 0 {
            return Err(Error::from(io::Error::from(io::ErrorKind::UnexpectedEof)));
        }

        if let Some((res, mut decoder)) = ctx.decode_head(&mut buf)? {
            // check if server sent connection close header.

            // *. If send_inner function produces error, Context has already set
            // connection type to ConnectionType::CloseForce. We trust the server response
            // to not produce another connection type that override it to any variant
            // other than ConnectionType::Close in this case and only this case.

            let mut is_close = ctx.is_connection_closed();

            if is_head_method {
                if !decoder.is_eof() {
                    // Server return a response body with head method.
                    // close the connection to drop the potential garbage data on wire.
                    is_close = true;
                }
                decoder = TransferCoding::eof();
            }

            return Ok((res, buf, decoder, is_close));
        }
    }
}

async fn send_inner<S, B, E>(
    stream: &mut S,
    mut encoder: TransferCoding,
    body: B,
    buf: &mut BytesMut,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    if !encoder.is_eof() {
        let mut body = pin!(body);

        // poll request body and encode.
        while let Some(bytes) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
            let bytes = bytes.map_err(BodyError::from)?;
            encoder.encode(bytes, buf);
            // we are not in a hurry here so write before handling next chunk.
            write_all_buf(&mut *stream, buf).await?;
        }

        // body is finished. encode eof and clean up.
        encoder.encode_eof(buf);

        write_all_buf(&mut *stream, buf).await?;
    }

    poll_fn(|cx| Pin::new(&mut *stream).poll_flush(cx))
        .await
        .map_err(Into::into)
}

async fn write_all_buf<S>(stream: &mut S, buf: &mut BytesMut) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let mut stream = Pin::new(stream);

    while buf.has_remaining() {
        let n = poll_fn(|cx| stream.as_mut().poll_write(cx, buf.chunk())).await?;
        buf.advance(n);
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::WriteZero));
        }
    }

    Ok(())
}
