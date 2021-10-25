pub(crate) mod proto;

use std::{error, io};

use futures_core::Stream;
use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{error::Error, http};

pub(crate) async fn send<S, B, T, E>(stream: &mut S, req: http::Request<B>) -> Result<(), Error>
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

    while let Some(item) = body.next().await {
        let item = item.map_err(|e| Error::Std(Box::new(e)))?;
        stream.write_all(item.as_ref()).await?;
    }

    stream.flush().await?;

    Err(io::Error::from(io::ErrorKind::Other).into())
}
