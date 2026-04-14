use core::{
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{
    body::{Body, Frame, SizeHint},
    bytes::Bytes,
    error::BodyError,
};

use super::{
    dispatcher::{Message, Shared},
    proto::frame::{settings, stream_id::StreamId},
};

pub struct RequestBody {
    stream_id: StreamId,
    size: SizeHint,
    ctx: Shared,
    /// Bytes consumed but not yet reported back as a WINDOW_UPDATE.
    /// Flushed as a single message when the channel has no more items
    /// ready, batching updates across consecutive chunks.
    pending_window: usize,
}

impl RequestBody {
    pub(super) fn new(stream_id: StreamId, size: SizeHint, ctx: Shared) -> Self {
        Self {
            stream_id,
            size,
            ctx,
            pending_window: 0,
        }
    }
}

impl Drop for RequestBody {
    fn drop(&mut self) {
        let mut inner = self.ctx.borrow_mut();
        let ci = &mut *inner;

        ci.request_body_drop(&self.stream_id);

        // Replenish any bytes consumed but not yet acknowledged. The stream
        // itself may already be gone (e.g. RST_STREAM), so only the connection
        // window is restored (stream 0).
        let size = mem::replace(&mut self.pending_window, 0);
        ci.queue.push_window_update(size);
    }
}

impl Body for RequestBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        let this = self.get_mut();

        let inner = &mut *this.ctx.borrow_mut();
        let stream = inner.flow.stream_map.get_mut(&this.stream_id).unwrap();
        if let Some(frame) = stream.recv.queue.pop_front(&mut inner.frame_buf) {
            if let Some(bytes) = frame.data_ref() {
                this.pending_window += bytes.len();

                // Flush when the remaining window would drop below 25% of the
                // initial size (i.e. 75% consumed). This mirrors nginx's
                // threshold, which is widely deployed and clients are tuned
                // to work well against it. It is more eager than the common
                // window/2 practice, reducing the chance of the peer stalling
                // while still batching small chunks effectively.
                if this.pending_window >= settings::DEFAULT_INITIAL_WINDOW_SIZE as usize * 3 / 4 {
                    let window = mem::replace(&mut this.pending_window, 0);
                    stream.recv.window += window;
                    inner.queue.push_window_update(window);
                    inner.queue.messages.push_back(Message::WindowUpdate {
                        stream_id: this.stream_id,
                        size: window,
                    });
                }
            }

            Poll::Ready(Some(Ok(frame)))
        } else if let Some(e) = stream.recv.take_error() {
            // Peer reset the stream while body was in progress.
            Poll::Ready(Some(Err(e.into())))
        } else if stream.recv.is_eof() {
            // END_STREAM received and queue drained: graceful EOF.
            Poll::Ready(None)
        } else {
            stream.recv.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }

    fn is_end_stream(&self) -> bool {
        let state = self.ctx.borrow();
        let stream = state.flow.stream_map.get(&self.stream_id).unwrap();
        stream.recv.is_eof() && stream.recv.queue.is_empty()
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.size
    }
}

impl From<RequestBody> for crate::body::RequestBody {
    fn from(body: RequestBody) -> Self {
        crate::body::RequestBody::H2(body)
    }
}
