use std::{error, io};

use bytes::BytesMut;
use futures_core::Stream;
use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use xitca_http::http;

use crate::h1::error::Error;

use super::context::Context;

pub(crate) async fn run<S, B, T, E>(stream: &mut S, req: http::Request<B>) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
    B: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
    E: error::Error + Send + Sync + 'static,
{
    let (parts, body) = req.into_parts();

    tokio::pin!(stream);

    let method = parts.method;

    tokio::pin!(body);

    let mut ctx = Context::new(parts.headers);
    let mut buf = BytesMut::new();

    while let Some(item) = body.next().await {
        let item = item.map_err(|e| Error::Std(Box::new(e)))?;
        stream.write_all(item.as_ref()).await?;
    }

    stream.flush().await?;

    let mut n = 0;

    let res = loop {
        n += stream.read_buf(&mut buf).await?;

        match ctx.decode(&mut buf)? {
            Some(res) => break res,
            None => continue,
        }
    };

    Err(io::Error::from(io::ErrorKind::Other).into())
}
