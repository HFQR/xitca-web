use core::task::{Context, Poll, Waker};

use std::io;

use crate::{body::SizeHint, bytes::Bytes, h2::util::Deque, http::HeaderMap};

use super::{
    error::Error,
    flow::{Frame, FrameBuffer},
    frame::reason::Reason,
    size::BodySize,
    window::{RecvWindow, SendWindow},
};

pub(super) struct Stream {
    queue: Deque,
    recv_waker: Option<Waker>,
    recv_window: RecvWindow,
    content_length: SizeHint,
    send_window: SendWindow,
    send_waker: Option<Waker>,
    recv_state: State,
    send_state: State,
    pending_error: Option<StreamError>,
}

impl Stream {
    pub(super) fn new(
        send_window: SendWindow,
        recv_window: RecvWindow,
        content_length: SizeHint,
        end_stream: bool,
    ) -> Self {
        let recv_state = if end_stream { State::Eof } else { State::Open };

        Self {
            queue: Deque::new(),
            recv_waker: None,
            recv_window,
            content_length,
            send_window,
            send_waker: None,
            recv_state,
            send_state: State::Open,
            pending_error: None,
        }
    }

    pub(super) fn try_recv_data(
        &mut self,
        buffer: &mut FrameBuffer,
        data: Bytes,
        flow_len: RecvWindow,
        end_stream: bool,
    ) -> Result<RecvData, Error> {
        self.recvable()?;

        let len = data.len();
        let padded_len = flow_len - RecvWindow::new(len as u32);

        let recv = match self.data_check(len, flow_len, end_stream) {
            Ok(_) => {
                if self.recv_state.is_open() {
                    // RFC 7540 §6.9.1: padding counts toward flow control. The caller
                    // auto-releases `padded_len` on both connection and stream windows
                    // since padding is not body-observable. Mirror that here so
                    // `recv.window` only paces the data portion via body consumption.
                    self.recv_window += padded_len;
                    self.recv_push_frame(buffer, Frame::Data(data), end_stream);
                    RecvData::Queued(padded_len)
                } else {
                    // Body dropped (Close) or error (Error): discard data, restore
                    // stream window so the caller can replenish via WINDOW_UPDATE
                    // and the peer can reach END_STREAM without stalling.
                    self.recv_window += flow_len;
                    RecvData::Discard(flow_len)
                }
            }
            Err(err) => {
                self.try_set_reset(err);
                RecvData::StreamReset(flow_len)
            }
        };

        Ok(recv)
    }

    pub(super) fn try_recv_trailers(
        &mut self,
        buffer: &mut FrameBuffer,
        trailers: HeaderMap,
        end_stream: bool,
    ) -> Result<RecvData, Error> {
        self.recvable()?;

        let recv = match self.trailers_check(end_stream) {
            Ok(_) => {
                if self.recv_state.is_open() {
                    self.recv_push_frame(buffer, Frame::Trailers(trailers), true);
                    RecvData::Queued(RecvWindow::ZERO)
                } else {
                    RecvData::Discard(RecvWindow::ZERO)
                }
            }
            Err(err) => {
                self.try_set_reset(err);
                RecvData::StreamReset(RecvWindow::ZERO)
            }
        };

        Ok(recv)
    }

    /// Pre-check that growing the send window by `delta` would not exceed
    /// MAX_INITIAL_WINDOW_SIZE. Used during SETTINGS_INITIAL_WINDOW_SIZE
    /// expansion before mutating any stream window. Caller guarantees `delta > 0`.
    pub(super) fn send_window_check(&self, delta: SendWindow) -> Result<(), StreamError> {
        let mut probe = self.send_window;
        probe.try_inc(delta).map_err(|_| StreamError::WindowUpdateOverflow)
    }

    /// Apply a peer WINDOW_UPDATE frame (always non-negative). Sets the
    /// stream as reset if the increment would overflow.
    pub(super) fn try_send_window_update(&mut self, incr: SendWindow, conn_window_positive: bool) {
        match self.send_window.try_inc(incr) {
            Ok(_) => {
                if conn_window_positive && self.send_window.is_positive() {
                    self.send_wake();
                }
            }
            Err(_) => self.try_set_reset(StreamError::WindowUpdateOverflow),
        }
    }

    /// Apply a SETTINGS_INITIAL_WINDOW_SIZE delta (may be negative) and wake
    /// the stream if the window is now positive.
    pub(super) fn send_window_update(&mut self, delta: SendWindow, conn_window_positive: bool) {
        self.send_window.apply_initial_delta(delta);
        if conn_window_positive && self.send_window.is_positive() {
            self.send_wake();
        }
    }

    pub(super) fn recv_window_update(&mut self, size: RecvWindow) {
        self.recv_window += size;
    }

    pub(super) fn poll_frame(
        &mut self,
        buffer: &mut FrameBuffer,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame, StreamError>>> {
        let opt = if let Some(frame) = self.queue.pop_front(buffer) {
            Some(Ok(frame))
        } else {
            match self.recv_state {
                State::Error => self.pending_error.map(Err),
                State::Eof => None,
                _ => {
                    self.recv_waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }
        };

        Poll::Ready(opt)
    }

    pub(super) fn poll_send_window(
        &mut self,
        req: SendWindow,
        conn_window: &mut SendWindow,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<SendWindow, StreamError>>> {
        let opt = match self.send_state {
            State::Error => self.pending_error.map(Err),
            State::Close => None,
            _ => {
                debug_assert!(req > SendWindow::ZERO);

                if !self.send_window.is_positive() || !conn_window.is_positive() {
                    self.send_waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }

                let aval = self.send_window.min(req).min(*conn_window);
                self.send_window -= aval;
                *conn_window -= aval;

                Some(Ok(aval))
            }
        };

        Poll::Ready(opt)
    }

    pub(super) fn promote_cancel_to_close_recv(&mut self) {
        if matches!(self.recv_state, State::Cancel) {
            self.recv_state = State::Close;
            self.try_revert_cancel_error();
        }
    }

    pub(super) fn close_send(&mut self) {
        self.send_state = State::Close;
    }

    pub(super) fn maybe_close_recv(&mut self, buffer: &mut FrameBuffer) -> RecvWindow {
        let mut window = RecvWindow::ZERO;

        while let Some(frame) = self.queue.pop_front(buffer) {
            if let Frame::Data(bytes) = frame {
                // Each queued Data was admitted via recv_window.checked_sub(len), and
                // recv_window is bounded by MAX_INITIAL_WINDOW_SIZE (2^31-1), so the
                // sum of unconsumed lens fits in u32.
                window += bytes.len() as u32;
            }
        }

        self.recv_window += window;

        match self.recv_state {
            State::Open => {
                self.recv_state = State::Cancel;
                self.try_set_pending_error(StreamError::NoError);
            }
            _ => {
                self.recv_state = State::Close;
            }
        }

        window
    }

    pub(super) fn try_remove(&mut self) -> TryRemove {
        match (&self.recv_state, &self.send_state) {
            (State::Close, State::Close) => match self.pending_error.take() {
                Some(err) if err.transportable() => TryRemove::ResetRemove(err),
                _ => TryRemove::Remove,
            },
            (State::Cancel, State::Close) => match self.pending_error.take() {
                Some(StreamError::NoError) => TryRemove::ResetKeep(StreamError::NoError),
                _ => TryRemove::Keep,
            },
            _ => TryRemove::Keep,
        }
    }

    /// Record a reset caused by a protocol error or server error.
    /// Sets recv error (so RequestBody sees it), send error (so response_task's
    /// send_data exits), and pending_reset (so the lifecycle sends RST_STREAM).
    pub(super) fn try_set_reset(&mut self, err: StreamError) {
        self.try_set_pending_error(err);
        self.recv_try_set_err();
        self.send_try_set_err();
    }

    /// Set an error on both sides without scheduling an outgoing RST_STREAM.
    /// Used for peer-initiated RST_STREAM where both RequestBody and
    /// response_task must observe the error and exit, but we must not echo
    /// back a RST_STREAM.
    pub(super) fn try_set_peer_reset(&mut self) {
        self.try_set_reset(StreamError::PeerReset);
    }

    pub(super) fn is_recv_end_stream(&self) -> bool {
        matches!(self.recv_state, State::Eof) && self.queue.is_empty()
    }

    fn recvable(&self) -> Result<(), Error> {
        match (&self.pending_error, &self.recv_state) {
            (Some(StreamError::PeerReset), _) => Err(Error::FrameAfterReset),
            (_, State::Eof) => Err(Error::FrameAfterEndStream),
            (_, _) => Ok(()),
        }
    }

    fn data_check(&mut self, len: usize, flow_len: RecvWindow, end_stream: bool) -> Result<(), StreamError> {
        // content-length accounts for data only; padding is not part of the message body.
        self.content_length
            .dec(len)
            .map_err(|_| StreamError::ContentLengthOverflow)?;

        if end_stream {
            self.ensure_zero()?;
        }

        // flow control accounts for the full frame payload (data + pad byte + padding).
        self.recv_window
            .checked_sub(flow_len)
            .map_err(|_| StreamError::FlowControlOverflow)
    }

    fn trailers_check(&self, end_stream: bool) -> Result<(), StreamError> {
        if !end_stream {
            // RFC 7540 §8.1: trailer HEADERS MUST carry END_STREAM.
            return Err(StreamError::TrailersNoEndStream);
        }
        self.ensure_zero()
    }

    fn ensure_zero(&self) -> Result<(), StreamError> {
        self.content_length
            .ensure_zero()
            .map_err(|_| StreamError::ContentLengthUnderflow)
    }

    fn try_set_pending_error(&mut self, err: StreamError) {
        self.pending_error = match (self.pending_error, err) {
            (_, StreamError::PeerReset) => Some(StreamError::PeerReset),
            (None | Some(StreamError::NoError), err) => Some(err),
            _ => return,
        }
    }

    fn try_revert_cancel_error(&mut self) {
        if matches!(self.pending_error, Some(StreamError::NoError)) {
            self.pending_error = None;
        }
    }

    fn recv_try_set_err(&mut self) {
        match self.recv_state {
            State::Open => {
                self.recv_state = State::Error;
                self.recv_wake();
            }
            State::Cancel => self.recv_state = State::Close,
            _ => {}
        }
    }

    fn recv_push_frame(&mut self, buffer: &mut FrameBuffer, frame: Frame, end_stream: bool) {
        self.queue.push_back(buffer, frame);
        if end_stream {
            self.recv_state = State::Eof;
        }
        self.recv_wake();
    }

    fn send_try_set_err(&mut self) {
        if self.send_state.is_open() {
            self.send_state = State::Error;
            self.send_wake();
        }
    }

    fn send_wake(&mut self) {
        if let Some(waker) = self.send_waker.take() {
            waker.wake();
        }
    }

    fn recv_wake(&mut self) {
        if let Some(waker) = self.recv_waker.take() {
            waker.wake();
        }
    }
}

// outcome of Stream::try_remove
pub(super) enum TryRemove {
    // keep Stream as is
    Keep,
    // graceful removal.
    Remove,
    // act the same as Keep variant.
    // AND observer must send Reason with Reset frame and send to peer
    ResetKeep(StreamError),
    // act the same as Remove variant.
    // AND observer must send Reason with Reset frame and send to peer
    ResetRemove(StreamError),
}

/// Result of [`Stream::try_recv_data`]. Each variant tells the caller
/// how to handle flow control after the frame has been processed.
///
/// The connection-level receive window is always decremented by the caller
/// *before* `try_recv_data` runs, so `Discard` and `StreamReset` both
/// carry the frame length for the caller to replenish it. Sizes reported
/// here are the full flow-controlled length (data + pad byte + padding).
pub(super) enum RecvData {
    /// Data was queued to the body reader. The caller must still auto-release
    /// the `padded_len` portion on both connection and stream windows — only
    /// the data portion is paced by `RequestBody::poll_frame` as the
    /// application consumes it.
    Queued(RecvWindow),
    /// Body was dropped (recv `Close`) or has a pending error (recv `Error`).
    /// Data has been discarded. The caller must replenish **both** the
    /// connection-level and stream-level receive windows by the returned
    /// flow-controlled length so the peer can reach END_STREAM without stalling.
    Discard(RecvWindow),
    /// A protocol error was detected (`pending_reset` and recv/send errors
    /// have been set on the stream). The caller must replenish the
    /// **connection-level** receive window only — the stream is being reset,
    /// so a stream-level WINDOW_UPDATE is unnecessary.
    StreamReset(RecvWindow),
}

// lifecycle state of Recv and Send
enum State {
    Open,
    // for recv only
    Cancel,
    // for recv only
    Eof,
    Error,
    // ready to be removed from stream_map
    Close,
}

impl State {
    fn is_open(&self) -> bool {
        matches!(self, State::Open)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum StreamError {
    TrailersNoEndStream,
    ContentLengthOverflow,
    ContentLengthUnderflow,
    FlowControlOverflow,
    NoError,
    /// WINDOW_UPDATE with zero increment (RFC 7540 §6.9.1).
    WindowUpdateZeroIncrement,
    /// WINDOW_UPDATE caused stream window overflow.
    WindowUpdateOverflow,
    /// Server-side error (service error, response body error, etc.).
    InternalError,
    /// Peer sent RST_STREAM.
    PeerReset,
    Io,
    GoAway,
}

impl StreamError {
    pub(crate) fn reason(&self) -> Reason {
        match self {
            Self::FlowControlOverflow | Self::WindowUpdateOverflow => Reason::FLOW_CONTROL_ERROR,
            Self::PeerReset | Self::NoError => Reason::NO_ERROR,
            Self::InternalError | Self::Io | Self::GoAway => Reason::INTERNAL_ERROR,
            _ => Reason::PROTOCOL_ERROR,
        }
    }

    // certain StreamError variants are not meant to be sent to peer
    fn transportable(&self) -> bool {
        !matches!(self, Self::PeerReset | Self::Io | Self::GoAway)
    }
}

impl From<StreamError> for io::Error {
    fn from(err: StreamError) -> Self {
        let msg = match err {
            StreamError::TrailersNoEndStream => "trailer HEADERS without END_STREAM",
            StreamError::ContentLengthOverflow => "content-length exceeded",
            StreamError::ContentLengthUnderflow => "content-length underflow at END_STREAM",
            StreamError::FlowControlOverflow => "stream flow control overflow",
            StreamError::NoError => "Recv stream canceled",
            StreamError::PeerReset => "h2 stream reset by peer",
            StreamError::WindowUpdateZeroIncrement => "WINDOW_UPDATE with zero increment",
            StreamError::WindowUpdateOverflow => "WINDOW_UPDATE caused window overflow",
            StreamError::InternalError => "internal error",
            StreamError::Io => return io::Error::new(io::ErrorKind::ConnectionAborted, "socket I/O error"),
            StreamError::GoAway => return io::Error::new(io::ErrorKind::ConnectionAborted, "connection is going away"),
        };
        io::Error::new(io::ErrorKind::InvalidData, msg)
    }
}
