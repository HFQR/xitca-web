use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::stream::{BoxStream, Stream};
use xitca_http::{bytes::Bytes, error::BodyError};

pub struct ResponseBody(pub BoxStream<'static, Result<Bytes, BodyError>>);

impl Stream for ResponseBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().0.as_mut().poll_next(cx)
    }
}
