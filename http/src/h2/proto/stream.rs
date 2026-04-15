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
    /// RST_STREAM reason to send when this stream is cleaned up by the
    /// lifecycle (RequestBody::drop / StreamGuard::drop). Set by the decode
    /// path (client protocol errors) or the response task (server errors).
    /// First reason wins — `get_or_insert` ensures a second error doesn't
    /// overwrite the original cause.
    pub(crate) pending_reset: Option<Reason>,
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
            pending_reset: None,
        }
    }

    pub(crate) fn try_get_recv<'a>(&'a mut self, buffer: &'a mut FrameBuffer) -> Result<RecvStream<'a>, Error> {
        RecvStream::try_new(self, buffer)
    }

    pub(crate) fn close_recv(&mut self, buffer: &mut FrameBuffer) {
        self.recv.set_close();
        self.recv.queue.clear(buffer);
    }

    pub(crate) fn is_recv_close(&self) -> bool {
        self.recv.is_close()
    }

    pub(crate) fn is_send_close(&self) -> bool {
        self.send.is_close()
    }

    pub(crate) fn is_close(&self) -> bool {
        // Both directions must be done before the entry can be removed.
        // Send: Closed or Error both count as done.
        // Recv: Close (body dropped before EOF) or Eof (natural completion)
        //       both count as done. Error does NOT — the body reader must
        //       take the error first.
        self.is_send_close() && (self.is_recv_close() || self.recv.is_eof())
    }

    /// Record a reset caused by a protocol error or server error.
    /// Sets recv error (so RequestBody sees it), send error (so StreamGuard's
    /// send_data exits), and pending_reset (so the lifecycle sends RST_STREAM).
    /// First reason wins via `get_or_insert`.
    pub(crate) fn set_reset(&mut self, err: io::Error, reason: Reason) {
        self.pending_reset.get_or_insert(reason);
        self.recv.try_set_error(err);
        self.send.set_error();
    }

    /// Set a recv-side error for the body reader without scheduling
    /// an outgoing RST_STREAM. Used for peer-initiated RST_STREAM where
    /// we need to notify RequestBody but must not echo back a reset.
    pub(crate) fn try_set_err(&mut self, err: io::Error) {
        self.recv.try_set_error(err)
    }
}

/// Guard type for receiving frames on a stream.
/// Borrows the whole Stream so it can set pending_reset, recv error,
/// and send error in a single operation on protocol violations.
pub(crate) struct RecvStream<'a> {
    stream: &'a mut Stream,
    buffer: &'a mut FrameBuffer,
}

impl<'a> RecvStream<'a> {
    fn try_new(stream: &'a mut Stream, buffer: &'a mut FrameBuffer) -> Result<Self, Error> {
        match stream.recv.state {
            // Open: normal receive path.
            // Close: body dropped before recv finished — data will be discarded.
            // Error: previous error pending for body reader — data will be discarded.
            RecvState::Open | RecvState::Close | RecvState::Error(_) => Ok(Self { stream, buffer }),
            // Eof: END_STREAM already received. Any further frames are a
            // protocol violation (RFC 7540 §5.1: "closed" state).
            RecvState::Eof => Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
        }
    }

    /// Try to receive a DATA frame. See [`RecvData`] for what the caller
    /// must do with each variant.
    pub(crate) fn try_recv_data(mut self, data: Bytes, end_stream: bool) -> RecvData {
        let len = data.len();

        if let Err(err) = self.data_check(len, end_stream) {
            self.stream.set_reset(err, Reason::PROTOCOL_ERROR);
            return RecvData::StreamReset(len);
        }

        if let Err(err) = self.try_window_dec(len) {
            self.stream.set_reset(err, Reason::FLOW_CONTROL_ERROR);
            return RecvData::StreamReset(len);
        }

        if self.stream.recv.is_open() {
            self.stream.recv.push_frame(self.buffer, Frame::Data(data), end_stream);
            RecvData::Queued
        } else {
            // Body dropped (Close) or error (Error): discard data, restore
            // stream window so the caller can replenish via WINDOW_UPDATE
            // and the peer can reach END_STREAM without stalling.
            self.stream.recv.window += len;
            RecvData::Discard(len)
        }
    }

    pub(crate) fn try_recv_trailers(self, trailers: HeaderMap, end_stream: bool) {
        if let Err(err) = self.trailers_check(end_stream) {
            self.stream.set_reset(err, Reason::PROTOCOL_ERROR);
        }

        if self.stream.recv.is_open() {
            self.stream
                .recv
                .push_frame(self.buffer, Frame::Trailers(trailers), true);
        }
    }

    fn data_check(&mut self, len: usize, end_stream: bool) -> io::Result<()> {
        if len == 0 && !end_stream {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "empty DATA without END_STREAM",
            ));
        }

        self.stream.recv.content_length.dec(len)?;

        if end_stream {
            self.stream.recv.content_length.ensure_zero()?;
        }

        Ok(())
    }

    fn trailers_check(&self, end_stream: bool) -> io::Result<()> {
        if !end_stream {
            // RFC 7540 §8.1: trailer HEADERS MUST carry END_STREAM.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "trailer HEADERS without END_STREAM",
            ));
        }
        self.stream.recv.content_length.ensure_zero()
    }

    fn try_window_dec(&mut self, len: usize) -> io::Result<()> {
        match self.stream.recv.window.checked_sub(len) {
            Some(window) => {
                self.stream.recv.window = window;
                Ok(())
            }
            None => Err(io::Error::new(io::ErrorKind::InvalidData, "flow control error")),
        }
    }
}

/// Result of [`RecvStream::try_recv_data`]. Each variant tells the caller
/// how to handle flow control after the frame has been processed.
///
/// The connection-level receive window is always decremented by the caller
/// *before* `try_recv_data` runs, so `Discard` and `StreamReset` both
/// carry the frame length for the caller to replenish it.
pub(crate) enum RecvData {
    /// Data was queued to the body reader. No window replenishment needed —
    /// `RequestBody::poll_frame` handles stream and connection windows as
    /// the application consumes the data.
    Queued,
    /// Body was dropped (recv `Close`) or has a pending error (recv `Error`).
    /// Data has been discarded. The caller must replenish **both** the
    /// connection-level and stream-level receive windows so the peer can
    /// reach END_STREAM without stalling.
    Discard(usize),
    /// A protocol error was detected (`pending_reset` and recv/send errors
    /// have been set on the stream). The caller must replenish the
    /// **connection-level** receive window only — the stream is being reset,
    /// so a stream-level WINDOW_UPDATE is unnecessary.
    StreamReset(usize),
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
    /// Recv-side state machine for the body reader.
    /// - Open:     body alive, accepting frames.
    /// - Eof:      END_STREAM received, body will see None on next poll.
    /// - Close:    body dropped before EOF (premature).
    /// - Error:    protocol error or peer reset — body will see Err on next poll.
    state: RecvState,
    /// RFC 7540 §8.1.2.6: tracks remaining expected DATA bytes.
    content_length: SizeHint,
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

    fn push_frame(&mut self, buffer: &mut FrameBuffer, frame: Frame, end_stream: bool) {
        self.queue.push_back(buffer, frame);
        if end_stream {
            self.state = RecvState::Eof;
        }
        self.wake();
    }

    /// Transition to Close. Only from Open — preserves Eof (natural
    /// completion) and Error (pending for body reader) so the lifecycle
    /// drops can distinguish why recv ended.
    fn set_close(&mut self) {
        if self.is_open() {
            self.state = RecvState::Close;
        }
    }

    pub(crate) fn is_close(&self) -> bool {
        matches!(self.state, RecvState::Close)
    }

    /// END_STREAM received or stream fully closed. Used by the body to
    /// detect EOF after draining the queue.
    pub(crate) fn is_eof(&self) -> bool {
        matches!(self.state, RecvState::Eof)
    }

    fn is_open(&self) -> bool {
        matches!(self.state, RecvState::Open)
    }

    /// Force recv to `Close`. Used on connection-level teardown.
    /// Does not overwrite `Error` — the error must be delivered to
    /// the body reader first.
    pub(crate) fn set_close_2(&mut self) {
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
    /// Stream was killed — peer RST_STREAM, client protocol error, or
    /// server error. StreamGuard's send_data observes this via
    /// `is_close()` and exits the body loop.
    Error,
    /// Send completed normally (END_STREAM sent or response task finished).
    Closed,
}

impl Send {
    pub(crate) fn is_close(&self) -> bool {
        !matches!(self.state, SendState::Open)
    }

    /// Signal that the stream was killed by an error. Wakes the send
    /// waker so StreamGuard's send_data poll exits promptly.
    pub(crate) fn set_error(&mut self) {
        if matches!(self.state, SendState::Open) {
            self.state = SendState::Error;
            self.wake();
        }
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
