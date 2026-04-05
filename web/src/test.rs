//! utilities for testing web application

use core::pin::pin;

use xitca_http::body::BodyExt;

use crate::{body::BodyStream, service::pipeline::PipelineE};

/// Collect request or response body to Vec.
pub async fn collect_body<B>(body: B) -> Result<Vec<u8>, B::Error>
where
    B: BodyStream,
{
    let mut body = pin!(body);

    let mut res = Vec::new();

    while let Some(frame) = body.as_mut().data().await {
        res.extend_from_slice(frame?.as_ref());
    }

    Ok(res)
}

pub type CollectStringError<E> = PipelineE<std::string::FromUtf8Error, E>;

/// Collect request or response body and parse it to String.
pub async fn collect_string_body<B>(body: B) -> Result<String, CollectStringError<B::Error>>
where
    B: BodyStream,
{
    let body = collect_body(body).await.map_err(CollectStringError::Second)?;
    String::from_utf8(body).map_err(CollectStringError::First)
}
