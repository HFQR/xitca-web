use core::{
    pin::Pin,
    task::{Context, Poll},
};

use std::io;

use crate::{
    body::{Body, Frame, SizeHint},
    bytes::Bytes,
    error::BodyError,
};

use super::{dispatcher::FLowControlClone, proto::frame::stream_id::StreamId};

pub struct RequestBody {
    id: StreamId,
    size: SizeHint,
    ctx: FLowControlClone,
    /// Bytes consumed but not yet reported back as a WINDOW_UPDATE.
    /// Flushed as a single message when the channel has no more items
    /// ready, batching updates across consecutive chunks.
    pending_window: usize,
}

impl RequestBody {
    pub(super) fn new(id: StreamId, size: SizeHint, ctx: FLowControlClone) -> Self {
        Self {
            id,
            size,
            ctx,
            pending_window: 0,
        }
    }
}

impl Drop for RequestBody {
    fn drop(&mut self) {
        self.ctx.borrow_mut().request_body_drop(self.id, self.pending_window);
    }
}

impl Body for RequestBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BodyError>>> {
        let this = self.get_mut();

        this.ctx
            .borrow_mut()
            .poll_stream_frame(&this.id, &mut this.pending_window, cx)
            .map_err(|err| BodyError::from(io::Error::from(err)))
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.ctx.borrow().is_recv_end_stream(&self.id)
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
