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
    dispatcher::{Message, Shared, StreamState},
    proto::{settings, stream_id::StreamId},
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
        // Service dropped RequestBody without consuming it to EOF. Set
        // RECV_CANCELED | BODY_CLOSED so poll_next returns None and the decode
        // path stops buffering, but keep the stream in the map until the peer
        // sends END_STREAM (RECV_CLOSED) so content-length enforcement still
        // runs. Also clear any pending error the caller dropped without reading.
        if let Some(state) = inner.flow.stream_map.get_mut(&self.stream_id) {
            if !state.recv_closed() {
                state.recv_state.queue.clear();
                state.recv_state.error = None;
                state.add_flag(StreamState::RECV_CANCELED);
            }

            // Only remove if RECV_CLOSED is also set (peer already done).
            if state.is_empty() {
                inner.flow.stream_map.remove(&self.stream_id);
            }
        }
        // Replenish any bytes consumed but not yet acknowledged. The stream
        // itself may already be gone (e.g. RST_STREAM), so only the connection
        // window is restored (stream 0).
        if self.pending_window > 0 {
            let size = mem::replace(&mut self.pending_window, 0);
            inner.queue.push_window_update(size);
        }
    }
}

impl Body for RequestBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        let this = self.get_mut();

        let mut inner = this.ctx.borrow_mut();
        match inner.flow.stream_map.get_mut(&this.stream_id) {
            Some(state) => {
                if let Some(frame) = state.recv_state.queue.pop_front() {
                    match frame {
                        Frame::Data(ref bytes) => {
                            this.pending_window += bytes.len();

                            // Flush when the remaining window would drop below 25% of the
                            // initial size (i.e. 75% consumed). This mirrors nginx's
                            // threshold, which is widely deployed and clients are tuned
                            // to work well against it. It is more eager than the common
                            // window/2 practice, reducing the chance of the peer stalling
                            // while still batching small chunks effectively.
                            if this.pending_window >= settings::DEFAULT_INITIAL_WINDOW_SIZE as usize * 3 / 4 {
                                let window = mem::replace(&mut this.pending_window, 0);
                                state.recv_state.window += window;
                                inner.queue.push_window_update(window);
                                inner.queue.messages.push_back(Message::WindowUpdate {
                                    stream_id: this.stream_id,
                                    size: window,
                                });
                            }
                            Poll::Ready(Some(Ok(frame)))
                        }
                        frame => Poll::Ready(Some(Ok(frame))),
                    }
                } else if let Some(e) = state.recv_state.error.take() {
                    // Peer reset the stream while body was in progress.
                    Poll::Ready(Some(Err(e)))
                } else if state.recv_closed() {
                    // Stream closed: graceful EOF (RECV_CLOSED).
                    Poll::Ready(None)
                } else {
                    state.recv_state.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
            // Stream fully removed (both sides done): clean EOF.
            None => Poll::Ready(None),
        }
    }

    fn is_end_stream(&self) -> bool {
        let inner = self.ctx.borrow();
        match inner.flow.stream_map.get(&self.stream_id) {
            Some(state) => state.recv_closed() && state.recv_state.queue.is_empty(),
            None => true,
        }
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
