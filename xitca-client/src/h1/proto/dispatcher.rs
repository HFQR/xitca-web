use std::io;

use bytes::{Buf, Bytes, BytesMut};
use futures_core::Stream;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use xitca_http::{error::BodyError, http};

use crate::{body::RequestBody, date::DateTimeHandle, h1::error::Error};

use super::context::Context;

pub(crate) async fn run<S, B, E>(
    stream: &mut S,
    date: DateTimeHandle<'_>,
    req: http::Request<RequestBody<B>>,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    let (parts, body) = req.into_parts();

    // TODO: make header limit configuarble.
    let mut ctx = Context::<128>::new(&date);
    let mut buf = BytesMut::new();

    // encode request head and return transfer encoding for request body
    let encoding = ctx.encode_head(&mut buf, parts, body.size())?;

    tokio::pin!(stream);

    // send request head for potential intermidiate handling like expect header.
    while buf.has_remaining() {
        stream.write_buf(&mut buf).await?;
    }

    if !encoding.is_eof() {
        tokio::pin!(body);

        while let Some(bytes) = body.as_mut().next().await {
            let bytes = bytes?;
            // encoding.encode(bytes, &mut buf)?;
            stream.write_all(&buf).await?;
        }
    }

    stream.flush().await?;

    let _ = loop {
        let n = stream.read_buf(&mut buf).await?;

        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }

        match ctx.decode_head(&mut buf)? {
            Some(res) => break res,
            None => continue,
        }
    };

    Ok(())
}
