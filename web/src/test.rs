use std::future::poll_fn;

use futures_core::stream::Stream;
use xitca_unsafe_collection::pin;

use crate::dev::service::pipeline::PipelineE;

/// Collect request or response body to Vec.
pub async fn collect_body<B, T, E>(body: B) -> Result<Vec<u8>, E>
where
    B: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    pin!(body);

    let mut res = Vec::new();

    while let Some(chunk) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
        res.extend_from_slice(chunk?.as_ref());
    }

    Ok(res)
}

pub type CollectStringError<E> = PipelineE<std::string::FromUtf8Error, E>;

/// Collect request or response body and parse it to String.
pub async fn collect_string_body<B, T, E>(body: B) -> Result<String, CollectStringError<E>>
where
    B: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    let body = collect_body(body).await.map_err(CollectStringError::Second)?;
    String::from_utf8(body).map_err(CollectStringError::First)
}
