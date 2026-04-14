use core::{mem, task::Waker};

use std::io;

use crate::{
    body::SizeHint,
    bytes::Bytes,
    h2::{
        dispatcher::{Frame, FrameBuffer},
        util::Deque,
    },
    http::HeaderMap,
};

use super::{
    error::Error,
    frame::{reason::Reason, settings},
    size::BodySize,
};

pub(crate) struct Stream {
    pub(crate) recv: Recv,
    pub(crate) send: Send,
}

impl Stream {
    #[allow(dead_code)]
    // TODO: strip response body for HEAD method request.
    const HEAD_METHOD: u8 = 1 << 3;

    pub(crate) fn new(send_window: i64, send_frame_size: usize, content_length: SizeHint, end_stream: bool) -> Self {
        let (window, state) = if end_stream {
            (0, RecvState::Eof)
        } else {
            (settings::DEFAULT_INITIAL_WINDOW_SIZE as usize, RecvState::Open)
        };

        Self {
            recv: Recv {
                queue: Deque::new(),
                waker: None,
                window,
                state,
                content_length,
            },
            send: Send {
                window: send_window,
                frame_size: send_frame_size,
                waker: None,
                state: SendState::Open,
            },
        }
    }

    pub(crate) fn try_get_recv<'a>(&'a mut self, buffer: &'a mut FrameBuffer) -> Result<RecvStream<'a>, Error> {
        RecvStream::try_new(&mut self.recv, buffer)
    }

    pub(crate) fn recv_closed(&self) -> bool {
        self.recv.recv_closed()
    }

    pub(crate) fn send_closed(&self) -> bool {
        self.send.send_closed()
    }

    pub(crate) fn is_empty(&self) -> bool {
        // Both directions must be closed before the entry can be removed.
        // Streams with a pending Error stay in the map until the body
        // reader takes the error.
        self.send_closed() && self.recv_closed()
    }

    /// Set a recv-side error for the body reader.
    ///
    /// Returns `true` when recv is already `Close` (body dropped),
    /// meaning the caller should handle full stream removal.
    pub(crate) fn try_set_err(&mut self, err: io::Error) {
        self.recv.try_set_error(err)
    }
}

// a guard type for Recv.
// RecvStream operate on specific RecvState combination
pub(crate) struct RecvStream<'a> {
    recv: &'a mut Recv,
    buffer: &'a mut FrameBuffer,
}

impl<'a> RecvStream<'a> {
    fn try_new(recv: &'a mut Recv, buffer: &'a mut FrameBuffer) -> Result<Self, Error> {
        match recv.state {
            // RecvState::Close can be arbitrary triggered at any time. eg:  before Stream received all frames
            // Therefore treat it the same as RecvState::Open
            RecvState::Open | RecvState::Close => Ok(Self { recv, buffer }),
            RecvState::Eof => Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
            RecvState::Error(_) => todo!("debate what to do with frame after error"),
        }
    }

    /// Try to receive a DATA frame. Returns `Ok(None)` when the body is alive
    /// and the data was queued. Returns `Ok(Some(data))` when the body has been
    /// dropped (recv closed) — the caller must replenish both connection and
    /// stream WINDOW_UPDATEs for the returned bytes so the peer can reach
    /// END_STREAM without stalling.
    pub(crate) fn try_recv_data(mut self, data: Bytes, end_stream: bool) -> Result<Option<Bytes>, Error> {
        let len = data.len();
        self.length_check(len, end_stream)?;

        if self.recv.is_open() {
            self.recv.queue.push_back(self.buffer, Frame::Data(data));
            if end_stream {
                self.recv.state = RecvState::Eof;
            }
            self.recv.wake();
            Ok(None)
        } else {
            // RecvState::Close is set before recv stream finished.
            debug_assert!(self.recv.recv_closed());
            self.recv.window += data.len();
            Ok(Some(data))
        }
    }

    pub(crate) fn try_recv_trailers(mut self, trailers: HeaderMap, end_stream: bool) -> Result<(), Error> {
        if !end_stream {
            // RFC 7540 §8.1: trailer HEADERS MUST carry END_STREAM.
            self.recv.try_set_error(io::Error::new(
                io::ErrorKind::InvalidData,
                "trailer HEADERS without END_STREAM",
            ));
            return Err(Error::Reset(Reason::PROTOCOL_ERROR));
        }

        self.ensure_zero()?;

        if self.recv.is_open() {
            self.recv.queue.push_back(self.buffer, Frame::Trailers(trailers));
            self.recv.state = RecvState::Eof;
            self.recv.wake();
        }

        Ok(())
    }

    fn length_check(&mut self, len: usize, end_stream: bool) -> Result<(), Error> {
        if len == 0 && !end_stream {
            self.recv.try_set_error(io::Error::new(
                io::ErrorKind::InvalidData,
                "empty DATA without END_STREAM",
            ));
            return Err(Error::Reset(Reason::PROTOCOL_ERROR));
        }

        if let Err(err) = self.recv.content_length.dec(len) {
            self.recv.try_set_error(err);
            return Err(Error::Reset(Reason::PROTOCOL_ERROR));
        }

        if end_stream {
            self.ensure_zero()?;
        }

        self.recv.window = self.recv.window.checked_sub(len).ok_or_else(|| {
            self.recv
                .try_set_error(io::Error::new(io::ErrorKind::InvalidData, "flow control error"));
            Error::Reset(Reason::FLOW_CONTROL_ERROR)
        })?;

        Ok(())
    }

    fn ensure_zero(&mut self) -> Result<(), Error> {
        self.recv.content_length.ensure_zero().map_err(|err| {
            self.recv.try_set_error(err);
            Error::Reset(Reason::PROTOCOL_ERROR)
        })
    }
}

pub(crate) struct Recv {
    /// Buffered DATA / Trailers frames for the request body, pushed by the
    /// decode path and drained by `RequestBody::poll_next`. Indices into
    /// the shared `ConnectionInner::frame_buf` slab.
    pub(crate) queue: Deque,
    /// Waker stored by `RequestBody::poll_next` when the queue is empty;
    /// woken by the decode path after pushing new data.
    pub(crate) waker: Option<Waker>,
    /// Remaining bytes the client may send on this stream (RFC 7540 §6.9).
    pub(crate) window: usize,
    /// Terminal error set when the stream is reset by the peer (RST_STREAM)
    /// while `RequestBody` is still alive. Delivered as `Err` on the next
    /// `poll_next` call so the caller knows the body was truncated.
    state: RecvState,
    /// RFC 7540 §8.1.2.6: tracks remaining expected DATA bytes.
    pub(crate) content_length: SizeHint,
}

enum RecvState {
    Open,
    Eof,
    Close,
    Error(io::Error),
}

impl Recv {
    fn try_set_error(&mut self, err: io::Error) {
        if self.is_open() {
            self.state = RecvState::Error(err);
            self.wake();
        }
    }

    pub(crate) fn close_recv(&mut self) {
        self.state = RecvState::Close;
    }

    pub(crate) fn recv_closed(&self) -> bool {
        matches!(self.state, RecvState::Close)
    }

    /// END_STREAM received or stream fully closed. Used by the body to
    /// detect EOF after draining the queue.
    pub(crate) fn is_eof(&self) -> bool {
        matches!(self.state, RecvState::Eof)
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self.state, RecvState::Open)
    }

    /// Force recv to `Close`. Used on connection-level teardown.
    /// Does not overwrite `Error` — the error must be delivered to
    /// the body reader first.
    pub(crate) fn set_close(&mut self) {
        match self.state {
            RecvState::Error(_) => {}
            _ => self.state = RecvState::Close,
        }
    }

    pub(crate) fn take_error(&mut self) -> Option<io::Error> {
        if matches!(self.state, RecvState::Error(_)) {
            let RecvState::Error(err) = mem::replace(&mut self.state, RecvState::Eof) else {
                unreachable!()
            };
            Some(err)
        } else {
            None
        }
    }

    pub(crate) fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

pub(crate) struct Send {
    /// Remaining send window for this stream. Signed because a SETTINGS change
    /// reducing INITIAL_WINDOW_SIZE can drive it negative; the stream must not
    /// send until WINDOW_UPDATE brings it back above zero (RFC 7540 §6.9.2).
    pub(crate) window: i64,
    pub(crate) frame_size: usize,
    pub(crate) waker: Option<Waker>,
    state: SendState,
}

enum SendState {
    Open,
    Closed,
}

impl Send {
    pub(crate) fn send_closed(&self) -> bool {
        matches!(self.state, SendState::Closed)
    }

    pub(crate) fn set_close(&mut self) {
        self.state = SendState::Closed;
    }

    pub(crate) fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}
