use core::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use ::h3::client::RequestStream;
use h3_quinn::BidiStream;

use crate::{
    body::{Body, BodyError, Frame},
    bytes::{Buf, Bytes},
};

type Stream = RequestStream<BidiStream<Bytes>, Bytes>;

/// Bidirectional streaming type for HTTP/3 responses.
///
/// Owns both the send and receive halves of the underlying h3
/// `RequestStream`, mirroring the structure of `h2::body::ResponseBody`.
pub struct ResponseBody {
    stream: Stream,
}

impl ResponseBody {
    pub(super) fn new(stream: Stream) -> Self {
        Self { stream }
    }

    #[cfg(feature = "grpc")]
    /// Send data to the server on the request half of this stream.
    ///
    /// This is an async operation because h3/QUIC flow control is
    /// inherently async (unlike h2's poll-based capacity reservation).
    pub(crate) async fn send_data(&mut self, buf: Bytes) -> Result<(), crate::h3::Error> {
        self.stream.send_data(buf).await.map_err(Into::into)
    }

    #[cfg(feature = "grpc")]
    /// Signal end-of-stream with an empty DATA frame.
    pub(crate) async fn finish(&mut self) -> Result<(), crate::h3::Error> {
        self.stream.finish().await.map_err(Into::into)
    }
}

impl Body for ResponseBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        let this = self.get_mut();
        let opt = match ready!(this.stream.poll_recv_data(cx)) {
            Ok(Some(bytes)) => Some(Ok(Frame::Data(Bytes::copy_from_slice(bytes.chunk())))),
            Err(e) => Some(Err(e.into())),
            Ok(None) => match ready!(this.stream.poll_recv_trailers(cx)) {
                Ok(Some(trailers)) => Some(Ok(Frame::Trailers(trailers))),
                Err(e) => Some(Err(e.into())),
                Ok(None) => None,
            },
        };
        Poll::Ready(opt)
    }
}
