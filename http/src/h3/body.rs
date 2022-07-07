use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::stream::{BoxStream, Stream};

use crate::{bytes::Bytes, error::BodyError};

/// Request body type for Http/3 specifically.
pub struct RequestBody(pub(super) BoxStream<'static, Result<Bytes, h3::Error>>);

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().0.as_mut().poll_next(cx).map_err(Into::into)
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        Self::H3(body)
    }
}
