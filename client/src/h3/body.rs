use std::{
    pin::Pin,
    task::{Context, Poll},
};

use xitca_http::body::{Body, Frame, SizeHint};
use xitca_http::{bytes::Bytes, error::BodyError};

pub type BoxBody<'a> = Pin<Box<dyn Body<Data = Bytes, Error = BodyError> + Send + Sync + 'a>>;

pub struct ResponseBody(pub BoxBody<'static>);

impl Body for ResponseBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        self.get_mut().0.as_mut().poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.0.size_hint()
    }
}
