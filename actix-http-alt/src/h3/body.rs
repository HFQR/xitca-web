use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::stream::{LocalBoxStream, Stream};

use crate::error::BodyError;

/// Request body type for Http/3 specifically.
pub struct RequestBody(pub(super) LocalBoxStream<'static, Result<Bytes, h3::Error>>);

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().0.as_mut().poll_next(cx).map_err(Into::into)
    }
}
