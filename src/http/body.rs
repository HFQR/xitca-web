use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::Stream;

use super::error::BodyError;

/// A unified request body type for different http protocols.
/// This enables one service type to handle multiple http protocols.
pub enum RequestBody {
    H2(super::h2::RequestBody),
}

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::H2(body) => Pin::new(body).poll_next(cx),
        }
    }
}
