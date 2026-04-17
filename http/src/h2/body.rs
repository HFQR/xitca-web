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
    STREAM_MUST_EXIST,
    dispatcher::{Message, Shared},
    proto::{frame::stream_id::StreamId, threshold::StreamRecvWindowThreshold},
};

pub struct RequestBody {
    stream_id: StreamId,
    size: SizeHint,
    ctx: Shared,
    threshold: StreamRecvWindowThreshold,
    /// Bytes consumed but not yet reported back as a WINDOW_UPDATE.
    /// Flushed as a single message when the channel has no more items
    /// ready, batching updates across consecutive chunks.
    pending_window: usize,
}

impl RequestBody {
    pub(super) fn new(stream_id: StreamId, size: SizeHint, ctx: Shared, threshold: StreamRecvWindowThreshold) -> Self {
        Self {
            stream_id,
            size,
            ctx,
            threshold,
            pending_window: 0,
        }
    }
}

impl Drop for RequestBody {
    fn drop(&mut self) {
        self.ctx
            .borrow_mut()
            .request_body_drop(self.stream_id, self.pending_window);
    }
}

impl Body for RequestBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        let this = self.get_mut();

        let flow = &mut *this.ctx.borrow_mut();
        let stream = flow.stream_map.get_mut(&this.stream_id).expect(STREAM_MUST_EXIST);

        stream.poll_frame(&mut flow.frame_buf, cx).map_ok(|frame| {
            if let Some(bytes) = frame.data_ref() {
                this.pending_window += bytes.len();

                if this.pending_window >= this.threshold {
                    let window = mem::replace(&mut this.pending_window, 0);
                    stream.recv.window += window;
                    flow.queue.connection_window_update(window);
                    flow.queue.messages.push_back(Message::WindowUpdate {
                        stream_id: this.stream_id,
                        size: window,
                    });
                }
            }
            frame
        })
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.ctx
            .borrow()
            .stream_map
            .get(&self.stream_id)
            .expect(STREAM_MUST_EXIST)
            .is_recv_end_stream()
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
