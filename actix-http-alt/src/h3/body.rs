use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::Stream;

use crate::error::BodyError;

/// Request body type for Http/3 specifically.
pub struct RequestBody;

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        log::warn!("Http/3 Request body handling is not implemented yet. Request body is dropped.");
        Poll::Ready(None)
    }
}
