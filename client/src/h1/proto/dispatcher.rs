use std::io;

use futures_core::Stream;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use xitca_http::{
    bytes::{Bytes, BytesMut},
    error::BodyError,
    h1::proto::{buf::FlatBuf, codec::TransferCoding},
    http::{
        self,
        header::{HeaderValue, HOST},
        Method,
    },
};

use crate::{body::RequestBody, date::DateTimeHandle, h1::Error};

use super::context::Context;

pub(crate) async fn send<S, B, E>(
    stream: &mut S,
    date: DateTimeHandle<'_>,
    mut req: http::Request<RequestBody<B>>,
) -> Result<(http::Response<()>, FlatBuf<{ 1024 * 1024 }>, TransferCoding, bool), Error>
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
    let mut buf = FlatBuf::<{ 1024 * 1024 }>::new();

    // encode request head and return transfer encoding for request body
    let encoder = ctx.encode_head(&mut buf, parts, body.size())?;

    // send request head for potential intermediate handling like expect header.
    stream.write_all_buf(&mut *buf).await?;

    // TODO: concurrent read write is needed in case server decide to do two way
    // streaming with very large body surpass socket buffer size.
    // (In rare case the server could starting streaming back resposne without read all the request body)

    // try to send request body.
    // continue to read response no matter the outcome.
    if send_inner(stream, encoder, body, &mut buf).await.is_err() {
        // an error indicate connection should be closed.
        ctx.set_close_on_error();
        // clear the buffer as there could be unfinished request data inside.
        buf.clear();
    }

    // read response head and get body decoder.
    loop {
        let n = stream.read_buf(&mut *buf).await?;

        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }

        match ctx.decode_head(&mut buf)? {
            Some((res, mut decoder)) => {
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
            None => continue,
        }
    }
}

async fn send_inner<S, B, E, const LIMIT: usize>(
    stream: &mut S,
    mut encoder: TransferCoding,
    body: RequestBody<B>,
    buf: &mut FlatBuf<LIMIT>,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    if !encoder.is_eof() {
        tokio::pin!(body);

        // poll request body and encode.
        while let Some(bytes) = body.as_mut().next().await {
            let bytes = bytes.map_err(BodyError::from)?;
            encoder.encode(bytes, buf);
            // we are not in a hurry here so read and flush before next chunk
            stream.write_all_buf(&mut **buf).await?;
            stream.flush().await?;
        }

        // body is finished. encode eof and clean up.
        encoder.encode_eof(buf);
        stream.write_all_buf(&mut **buf).await?;
    }

    stream.flush().await?;

    Ok(())
}
