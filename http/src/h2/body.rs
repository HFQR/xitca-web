use core::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use futures_core::stream::Stream;
use h2::RecvStream;

use crate::{bytes::Bytes, error::BodyError};

/// Request body type for Http/2 specifically.
pub struct RequestBody {
    end_stream: bool,
    stream: RecvStream,
}

impl Stream for RequestBody {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();

        if this.end_stream {
            return Poll::Ready(None);
        }

        let res = ready!(this.stream.poll_data(cx)?);

        this.end_stream = this.stream.is_end_stream();

        match res {
            Some(bytes) if bytes.is_empty() => self.poll_next(cx),
            Some(bytes) => {
                this.stream
                    .flow_control()
                    .release_capacity(bytes.len())
                    .expect("releasing the same amount of received data should never fail");
                Poll::Ready(Some(Ok(bytes)))
            }
            None => Poll::Ready(None),
        }
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        Self::H2(body)
    }
}

impl From<RecvStream> for RequestBody {
    fn from(stream: RecvStream) -> Self {
        RequestBody {
            end_stream: false,
            stream,
        }
    }
}

// Skip h2::body::RequestBody type and convert to crate level RequestBody directly
impl From<RecvStream> for crate::body::RequestBody {
    fn from(stream: RecvStream) -> Self {
        RequestBody::from(stream).into()
    }
}
