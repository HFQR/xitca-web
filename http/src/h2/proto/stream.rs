use core::task::Waker;

use std::{io, mem};

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
    pending_reset: PendingReset,
}

impl Stream {
    #[allow(dead_code)]
    // TODO: strip response body for HEAD method request.
    const HEAD_METHOD: u8 = 1 << 3;

    pub(crate) fn new(send_window: i64, send_frame_size: usize, content_length: SizeHint, end_stream: bool) -> Self {
        let (window, state) = if end_stream {
            (0, State::Eof)
        } else {
            (settings::DEFAULT_INITIAL_WINDOW_SIZE as usize, State::Open)
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
                state: State::Open,
            },
            pending_reset: PendingReset::None,
        }
    }

    pub(crate) fn try_get_recv<'a>(&'a mut self, buffer: &'a mut FrameBuffer) -> Result<RecvStream<'a>, Error> {
        RecvStream::try_new(self, buffer)
    }

    // may close or cancel the recv side of stream
    // behavior depend on current state of Recv.
    // State::Open -> State::Cancel. return with RecvClose::Cancel
    // State::<non_OPEN> -> state::Close. return with RecvClose::Close
    pub(crate) fn maybe_close_recv(&mut self, buffer: &mut FrameBuffer) -> RecvClose {
        self.recv.set_close();

        let mut window = 0;

        while let Some(frame) = self.recv.queue.pop_front(buffer) {
            if let Frame::Data(bytes) = frame {
                window += bytes.len();
            }
        }

        self.recv.window += window;

        if self.recv.state.is_close() {
            RecvClose::Close(window)
        } else {
            RecvClose::Cancel(window)
        }
    }

    pub(crate) fn is_send_close(&self) -> bool {
        self.send.state.is_close()
    }

    pub(crate) fn try_remove(&mut self) -> Option<Remove> {
        (self.is_send_close() && self.recv.state.is_close()).then(|| match self.pending_reset.take() {
            Some(reason) => Remove::Reset(reason),
            None => Remove::Graceful,
        })
    }

    /// Record a reset caused by a protocol error or server error.
    /// Sets recv error (so RequestBody sees it), send error (so StreamGuard's
    /// send_data exits), and pending_reset (so the lifecycle sends RST_STREAM).
    /// First reason wins via `get_or_insert`.
    pub(crate) fn set_reset(&mut self, err: StreamError) {
        self.pending_reset.try_set_local(err.reason());
        self.recv.try_set_err(err);
        self.send.try_set_err(err);
    }

    /// Set an error on both sides without scheduling an outgoing RST_STREAM.
    /// Used for peer-initiated RST_STREAM where both RequestBody and
    /// StreamGuard must observe the error and exit, but we must not echo
    /// back a RST_STREAM.
    pub(crate) fn try_set_peer_reset(&mut self) {
        self.pending_reset.try_set_peer();
        self.recv.try_set_err(StreamError::PeerReset);
        self.send.try_set_err(StreamError::PeerReset);
    }

    pub(crate) fn take_recv_err(&self) -> Option<io::Error> {
        self.recv.state.take_error()
    }

    pub(crate) fn take_send_err(&self) -> Option<io::Error> {
        self.send.state.take_error()
    }
}

// outcome of Stream::maybe_close_recv
pub(crate) enum RecvClose {
    // Recv is locally canceled but peer still consider it's open
    // observer must keep updating connection window and stream window to keep
    // stream going
    Cancel(usize),
    // Recv is locally closed
    // observer must keep updating connection window
    Close(usize),
}

// outcome of Stream::try_remove
pub(crate) enum Remove {
    // graceful removal nothing should be done
    Graceful,
    // observer must send Reason with Reset frame and send to peer
    Reset(Reason),
}

// Guard type for receiving frames on a stream.
// Borrows the whole Stream so it can set pending_reset, recv error,
// and send error in a single operation on protocol violations.
pub(crate) struct RecvStream<'a> {
    stream: &'a mut Stream,
    buffer: &'a mut FrameBuffer,
}

impl<'a> RecvStream<'a> {
    fn try_new(stream: &'a mut Stream, buffer: &'a mut FrameBuffer) -> Result<Self, Error> {
        match (&stream.pending_reset, &stream.recv.state) {
            (PendingReset::Peer, _) => Err(Error::GoAway(Reason::STREAM_CLOSED)),
            (_, State::Eof) => Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
            (_, _) => Ok(Self { stream, buffer }),
        }
    }

    /// Try to receive a DATA frame. See [`RecvData`] for what the caller
    /// must do with each variant.
    pub(crate) fn try_recv_data(mut self, data: Bytes, end_stream: bool) -> RecvData {
        let len = data.len();

        if let Err(err) = self.data_check(len, end_stream) {
            self.stream.set_reset(err);
            return RecvData::StreamReset(len);
        }

        if self.stream.recv.state.is_open() {
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

    pub(crate) fn try_recv_trailers(self, trailers: HeaderMap, end_stream: bool) -> RecvData {
        if let Err(err) = self.trailers_check(end_stream) {
            self.stream.set_reset(err);
            return RecvData::StreamReset(0);
        }

        if self.stream.recv.state.is_open() {
            self.stream
                .recv
                .push_frame(self.buffer, Frame::Trailers(trailers), true);
            RecvData::Queued
        } else {
            RecvData::Discard(0)
        }
    }

    fn data_check(&mut self, len: usize, end_stream: bool) -> Result<(), StreamError> {
        if len == 0 && !end_stream {
            return Err(StreamError::EmptyDataNoEndStream);
        }

        self.stream
            .recv
            .content_length
            .dec(len)
            .map_err(|_| StreamError::ContentLengthOverflow)?;

        if end_stream {
            self.ensure_zero()?;
        }

        self.try_window_dec(len)
    }

    fn trailers_check(&self, end_stream: bool) -> Result<(), StreamError> {
        if !end_stream {
            // RFC 7540 §8.1: trailer HEADERS MUST carry END_STREAM.
            return Err(StreamError::TrailersNoEndStream);
        }
        self.ensure_zero()
    }

    fn try_window_dec(&mut self, len: usize) -> Result<(), StreamError> {
        match self.stream.recv.window.checked_sub(len) {
            Some(window) => {
                self.stream.recv.window = window;
                Ok(())
            }
            None => Err(StreamError::FlowControlOverflow),
        }
    }

    fn ensure_zero(&self) -> Result<(), StreamError> {
        self.stream
            .recv
            .content_length
            .ensure_zero()
            .map_err(|_| StreamError::ContentLengthUnderflow)
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
    pub(crate) queue: Deque,
    pub(crate) waker: Option<Waker>,
    pub(crate) window: usize,
    state: State,
    content_length: SizeHint,
}

impl Recv {
    fn try_set_err(&mut self, err: StreamError) {
        if self.state.is_open() {
            self.state = State::Error(err);
            self.wake();
        }
    }

    fn push_frame(&mut self, buffer: &mut FrameBuffer, frame: Frame, end_stream: bool) {
        self.queue.push_back(buffer, frame);
        if end_stream {
            self.state = State::Eof;
        }
        self.wake();
    }

    fn set_close(&mut self) {
        self.state = match self.state {
            State::Open => State::Cancel,
            _ => State::Close,
        }
    }

    /// END_STREAM received or stream fully closed. Used by the body to
    /// detect EOF after draining the queue.
    pub(crate) fn is_eof(&self) -> bool {
        matches!(self.state, State::Eof)
    }

    /// Force recv to `Close`. Used on connection-level teardown.
    /// Does not overwrite `Error` — the error must be delivered to
    /// the body reader first.
    pub(crate) fn set_close_2(&mut self) {
        match self.state {
            State::Error(_) => {}
            _ => self.state = State::Close,
        }
    }

    fn wake(&mut self) {
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
    state: State,
}

impl Send {
    fn try_set_err(&mut self, err: StreamError) {
        if self.state.is_open() {
            self.state = State::Error(err);
            self.wake();
        }
    }

    pub(crate) fn set_close(&mut self) {
        self.state = State::Close;
    }

    pub(crate) fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

enum PendingReset {
    None,
    Peer,
    Local(Reason),
}

impl PendingReset {
    fn try_set_peer(&mut self) {
        if matches!(self, Self::None) {
            *self = Self::Peer;
        }
    }

    fn try_set_local(&mut self, reason: Reason) {
        if matches!(self, Self::None) {
            *self = Self::Local(reason);
        }
    }

    fn take(&mut self) -> Option<Reason> {
        match mem::replace(self, PendingReset::None) {
            PendingReset::Local(reason) => Some(reason),
            _ => None,
        }
    }
}

// lifecycle state of Recv and Send
enum State {
    Open,
    // for recv only
    Cancel,
    // for recv only
    Eof,
    Error(StreamError),
    // ready to be removed from stream_map
    Close,
}

impl State {
    fn is_open(&self) -> bool {
        matches!(self, State::Open)
    }

    fn is_close(&self) -> bool {
        matches!(self, State::Close)
    }

    fn take_error(&self) -> Option<io::Error> {
        match *self {
            State::Error(err) => Some(err.into()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum StreamError {
    EmptyDataNoEndStream,
    TrailersNoEndStream,
    ContentLengthOverflow,
    ContentLengthUnderflow,
    FlowControlOverflow,
    /// Peer sent RST_STREAM.
    PeerReset,
    /// WINDOW_UPDATE with zero increment (RFC 7540 §6.9.1).
    WindowUpdateZeroIncrement,
    /// WINDOW_UPDATE caused stream window overflow.
    WindowUpdateOverflow,
    /// Server-side error (service error, response body error, etc.).
    ServerError,
}

impl StreamError {
    pub(crate) fn reason(&self) -> Reason {
        match self {
            Self::FlowControlOverflow | Self::WindowUpdateOverflow => Reason::FLOW_CONTROL_ERROR,
            Self::PeerReset => Reason::NO_ERROR,
            Self::ServerError => Reason::INTERNAL_ERROR,
            _ => Reason::PROTOCOL_ERROR,
        }
    }
}

impl From<StreamError> for io::Error {
    fn from(err: StreamError) -> Self {
        let msg = match err {
            StreamError::EmptyDataNoEndStream => "empty DATA without END_STREAM",
            StreamError::TrailersNoEndStream => "trailer HEADERS without END_STREAM",
            StreamError::ContentLengthOverflow => "content-length exceeded",
            StreamError::ContentLengthUnderflow => "content-length underflow at END_STREAM",
            StreamError::FlowControlOverflow => "stream flow control overflow",
            StreamError::PeerReset => "h2 stream reset by peer",
            StreamError::WindowUpdateZeroIncrement => "WINDOW_UPDATE with zero increment",
            StreamError::WindowUpdateOverflow => "WINDOW_UPDATE caused window overflow",
            StreamError::ServerError => "server error",
        };
        io::Error::new(io::ErrorKind::InvalidData, msg)
    }
}
