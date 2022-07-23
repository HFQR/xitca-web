use std::{
    pin::Pin,
    task::{Context, Poll},
};

use xitca_http::{bytes::Bytes, error::BodyError, h2::body::RequestBody};

use futures_core::stream::Stream;

/// Though the naming is ResponseBody. It's actually a bi-directional
/// streaming type that able to add additional stream message to server.
pub struct ResponseBody {
    // TODO: use new type and import from xitca_http?
    #[allow(dead_code)]
    tx: h2::SendStream<Bytes>,
    /// When used by client [RequestBody] is used as part of response body.
    rx: RequestBody,
}

impl ResponseBody {
    pub(super) fn new(tx: h2::SendStream<Bytes>, rx: RequestBody) -> Self {
        Self { tx, rx }
    }
}

impl Stream for ResponseBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().rx).poll_next(cx)
    }
}
