use std::{io, pin::Pin};

use futures_core::Stream;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use xitca_http::{
    bytes::Bytes,
    error::BodyError,
    h1::proto::{buf::FlatBuf, codec::TransferCoding, context::ConnectionType},
    http::{self, Method},
};

use crate::{body::RequestBody, date::DateTimeHandle, h1::Error};

use super::context::Context;

pub(crate) async fn send<S, B, E>(
    stream: &mut S,
    date: DateTimeHandle<'_>,
    req: http::Request<RequestBody<B>>,
) -> Result<
    (
        http::Response<()>,
        FlatBuf<{ 1024 * 1024 }>,
        Option<TransferCoding>,
        bool,
    ),
    Error,
>
where
    S: AsyncRead + AsyncWrite + Unpin,
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    let is_head_method = *req.method() == Method::HEAD;

    let (parts, body) = req.into_parts();

    // TODO: make const generic params configurable.
    let mut ctx = Context::<128>::new(&date);
    let mut buf = FlatBuf::<{ 1024 * 1024 }>::new();

    // encode request head and return transfer encoding for request body
    let mut encoding = ctx.encode_head(&mut buf, parts, body.size())?;

    let mut stream = Pin::new(stream);

    // send request head for potential intermediate handling like expect header.
    stream.write_all_buf(&mut *buf).await?;

    if !encoding.is_eof() {
        tokio::pin!(body);

        // poll request body and encode.
        while let Some(bytes) = body.as_mut().next().await {
            let bytes = bytes?;
            encoding.encode(bytes, &mut buf);
            // we are not in a hurry here so read and flush before next chunk
            stream.write_all_buf(&mut *buf).await?;
            stream.flush().await?;
        }

        // body is finished. encode eof and clean up.
        encoding.encode_eof(&mut buf);
        stream.write_all_buf(&mut *buf).await?;
    }

    stream.flush().await?;

    // read response head and get body decoder.
    loop {
        let n = stream.read_buf(&mut *buf).await?;

        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }

        match ctx.decode_head(&mut buf)? {
            Some((res, decoder)) => {
                let decoder = match (is_head_method, decoder.is_eof()) {
                    (false, false) => Some(decoder),
                    _ => None,
                };

                let is_close = matches!(ctx.ctype(), ConnectionType::Close);

                return Ok((res, buf, decoder, is_close));
            }
            None => continue,
        }
    }
}
