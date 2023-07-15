use core::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::stream::Stream;
use h2::RecvStream;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{bytes::Bytes, error::BodyError};

/// Request body type for Http/2 specifically.
pub struct RequestBody(RecvStream);

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let stream = &mut self.get_mut().0;

        stream.poll_data(cx).map(|opt| {
            opt.map(|res| {
                let bytes = res?;
                stream.flow_control().release_capacity(bytes.len())?;

                Ok(bytes)
            })
        })
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        Self::H2(body)
    }
}

impl From<RecvStream> for RequestBody {
    fn from(stream: RecvStream) -> Self {
        RequestBody(stream)
    }
}

// Skip h2::body::RequestBody type and convert to crate level RequestBody directly
impl From<RecvStream> for crate::body::RequestBody {
    fn from(stream: RecvStream) -> Self {
        Self::H2(RequestBody(stream))
    }
}

/// Request body type for Http/2 specifically.
pub struct RequestBodyV2(UnboundedReceiver<Result<Bytes, BodyError>>);

#[cfg(feature = "io-uring")]
pub type RequestBodySender = tokio::sync::mpsc::UnboundedSender<Result<Bytes, BodyError>>;

impl RequestBodyV2 {
    #[cfg(feature = "io-uring")]
    pub(super) fn new_pair() -> (Self, RequestBodySender) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Self(rx), tx)
    }
}

impl Stream for RequestBodyV2 {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().0.poll_recv(cx)
    }
}
