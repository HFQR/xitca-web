use core::{
    cmp,
    task::{Context, Poll, Waker},
};

use std::io;

use crate::{
    body::SizeHint,
    bytes::Bytes,
    error::BodyError,
    h2::{
        dispatcher::{Frame, FrameBuffer},
        util::Deque,
    },
    http::HeaderMap,
};

use super::{error::Error, frame::reason::Reason, size::BodySize};

pub(crate) struct Stream {
    pub(crate) recv: Recv,
    pub(crate) send: Send,
    pending_error: Option<StreamError>,
}

impl Stream {
    pub(crate) fn new(
        send_window: i64,
        send_frame_size: usize,
        recv_window: usize,
        content_length: SizeHint,
        end_stream: bool,
    ) -> Self {
        let state = if end_stream { State::Eof } else { State::Open };

        Self {
            recv: Recv {
                queue: Deque::new(),
                waker: None,
                window: recv_window,
                state,
                content_length,
            },
            send: Send {
                window: send_window,
                frame_size: send_frame_size,
                waker: None,
                state: State::Open,
            },
            pending_error: None,
        }
    }

    pub(crate) fn try_recv_data(
        &mut self,
        buffer: &mut FrameBuffer,
        data: Bytes,
        flow_len: usize,
        end_stream: bool,
    ) -> Result<RecvData, Error> {
        self.recvable()?;

        let len = data.len();
        let padded_len = flow_len - len;

        let recv = match self.data_check(len, flow_len, end_stream) {
            Ok(_) => {
                if self.recv.state.is_open() {
                    // RFC 7540 §6.9.1: padding counts toward flow control. The caller
                    // auto-releases `padded_len` on both connection and stream windows
                    // since padding is not body-observable. Mirror that here so
                    // `recv.window` only paces the data portion via body consumption.
                    self.recv.window += padded_len;
                    self.recv.push_frame(buffer, Frame::Data(data), end_stream);
                    RecvData::Queued(padded_len)
                } else {
                    // Body dropped (Close) or error (Error): discard data, restore
                    // stream window so the caller can replenish via WINDOW_UPDATE
                    // and the peer can reach END_STREAM without stalling.
                    self.recv.window += flow_len;
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

    pub(crate) fn try_recv_trailers(
        &mut self,
        buffer: &mut FrameBuffer,
        trailers: HeaderMap,
        end_stream: bool,
    ) -> Result<RecvData, Error> {
        self.recvable()?;

        let recv = match self.trailers_check(end_stream) {
            Ok(_) => {
                if self.recv.state.is_open() {
                    self.recv.push_frame(buffer, Frame::Trailers(trailers), true);
                    RecvData::Queued(0)
                } else {
                    RecvData::Discard(0)
                }
            }
            Err(err) => {
                self.try_set_reset(err);
                RecvData::StreamReset(0)
            }
        };

        Ok(recv)
    }

    pub(crate) fn poll_frame(
        &mut self,
        buffer: &mut FrameBuffer,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame, BodyError>>> {
        let opt = if let Some(frame) = self.recv.queue.pop_front(buffer) {
            Some(Ok(frame))
        } else {
            match self.recv.state {
                State::Error => self.pending_error.map(|err| Err(BodyError::from(io::Error::from(err)))),
                State::Eof => None,
                _ => {
                    self.recv.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }
        };

        Poll::Ready(opt)
    }

    pub(crate) fn poll_send_window(
        &mut self,
        len: usize,
        window: usize,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<usize, StreamError>>> {
        let opt = match self.send.state {
            State::Error => self.pending_error.map(Err),
            State::Close => None,
            _ => {
                if len > 0 && (window == 0 || self.send.window <= 0) {
                    self.send.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }

                let len = cmp::min(len, self.send.frame_size);
                let aval = cmp::min(self.send.window as usize, window);
                let aval = cmp::min(aval, len);
                self.send.window -= aval as i64;

                Some(Ok(aval))
            }
        };

        Poll::Ready(opt)
    }

    pub(crate) fn promote_cancel_to_close_recv(&mut self) {
        if matches!(self.recv.state, State::Cancel) {
            self.recv.state = State::Close;
            self.try_revert_cancel_error();
        }
    }

    pub(crate) fn close_send(&mut self) {
        self.send.set_close();
    }

    pub(crate) fn maybe_close_recv(&mut self, buffer: &mut FrameBuffer) -> usize {
        let mut window = 0;

        while let Some(frame) = self.recv.queue.pop_front(buffer) {
            if let Frame::Data(bytes) = frame {
                window += bytes.len();
            }
        }

        self.recv.window += window;

        match self.recv.state {
            State::Open => {
                self.recv.state = State::Cancel;
                self.try_set_pending_error(StreamError::NoError);
            }
            _ => {
                self.recv.state = State::Close;
            }
        }

        window
    }

    pub(crate) fn try_remove(&mut self) -> TryRemove {
        match (&self.recv.state, &self.send.state) {
            (State::Close, State::Close) => match self.pending_error.take() {
                Some(err) if !matches!(err, StreamError::PeerReset) => TryRemove::ResetRemove(err),
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
    pub(crate) fn try_set_reset(&mut self, err: StreamError) {
        self.try_set_pending_error(err);
        self.recv.try_set_err();
        self.send.try_set_err();
    }

    /// Set an error on both sides without scheduling an outgoing RST_STREAM.
    /// Used for peer-initiated RST_STREAM where both RequestBody and
    /// response_task must observe the error and exit, but we must not echo
    /// back a RST_STREAM.
    pub(crate) fn try_set_peer_reset(&mut self) {
        self.try_set_reset(StreamError::PeerReset);
    }

    pub(crate) fn is_recv_end_stream(&self) -> bool {
        self.recv.is_eof() && self.recv.queue.is_empty()
    }

    fn recvable(&self) -> Result<(), Error> {
        match (&self.pending_error, &self.recv.state) {
            (Some(StreamError::PeerReset), _) => Err(Error::FrameAfterReset),
            (_, State::Eof) => Err(Error::FrameAfterEndStream),
            (_, _) => Ok(()),
        }
    }

    fn data_check(&mut self, len: usize, flow_len: usize, end_stream: bool) -> Result<(), StreamError> {
        if len == 0 && !end_stream {
            return Err(StreamError::EmptyDataNoEndStream);
        }

        // content-length accounts for data only; padding is not part of the message body.
        self.recv
            .content_length
            .dec(len)
            .map_err(|_| StreamError::ContentLengthOverflow)?;

        if end_stream {
            self.ensure_zero()?;
        }

        // flow control accounts for the full frame payload (data + pad byte + padding).
        self.try_window_dec(flow_len)
    }

    fn trailers_check(&self, end_stream: bool) -> Result<(), StreamError> {
        if !end_stream {
            // RFC 7540 §8.1: trailer HEADERS MUST carry END_STREAM.
            return Err(StreamError::TrailersNoEndStream);
        }
        self.ensure_zero()
    }

    fn try_window_dec(&mut self, len: usize) -> Result<(), StreamError> {
        match self.recv.window.checked_sub(len) {
            Some(window) => {
                self.recv.window = window;
                Ok(())
            }
            None => Err(StreamError::FlowControlOverflow),
        }
    }

    fn ensure_zero(&self) -> Result<(), StreamError> {
        self.recv
            .content_length
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
}

// outcome of Stream::try_remove
pub(crate) enum TryRemove {
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
pub(crate) enum RecvData {
    /// Data was queued to the body reader. The caller must still auto-release
    /// the `padded_len` portion on both connection and stream windows — only
    /// the data portion is paced by `RequestBody::poll_frame` as the
    /// application consumes it.
    Queued(usize),
    /// Body was dropped (recv `Close`) or has a pending error (recv `Error`).
    /// Data has been discarded. The caller must replenish **both** the
    /// connection-level and stream-level receive windows by the returned
    /// flow-controlled length so the peer can reach END_STREAM without stalling.
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
    fn try_set_err(&mut self) {
        match self.state {
            State::Open => {
                self.state = State::Error;
                self.wake();
            }
            State::Cancel => self.state = State::Close,
            _ => {}
        }
    }

    fn push_frame(&mut self, buffer: &mut FrameBuffer, frame: Frame, end_stream: bool) {
        self.queue.push_back(buffer, frame);
        if end_stream {
            self.state = State::Eof;
        }
        self.wake();
    }

    fn is_eof(&self) -> bool {
        matches!(self.state, State::Eof)
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
    fn try_set_err(&mut self) {
        if self.state.is_open() {
            self.state = State::Error;
            self.wake();
        }
    }

    fn set_close(&mut self) {
        self.state = State::Close;
    }

    pub(crate) fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
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
    EmptyDataNoEndStream,
    TrailersNoEndStream,
    ContentLengthOverflow,
    ContentLengthUnderflow,
    FlowControlOverflow,
    NoError,
    /// Peer sent RST_STREAM.
    PeerReset,
    /// WINDOW_UPDATE with zero increment (RFC 7540 §6.9.1).
    WindowUpdateZeroIncrement,
    /// WINDOW_UPDATE caused stream window overflow.
    WindowUpdateOverflow,
    /// Server-side error (service error, response body error, etc.).
    InternalError,
}

impl StreamError {
    pub(crate) fn reason(&self) -> Reason {
        match self {
            Self::FlowControlOverflow | Self::WindowUpdateOverflow => Reason::FLOW_CONTROL_ERROR,
            Self::PeerReset | Self::NoError => Reason::NO_ERROR,
            Self::InternalError => Reason::INTERNAL_ERROR,
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
            StreamError::NoError => "Recv stream canceled",
            StreamError::PeerReset => "h2 stream reset by peer",
            StreamError::WindowUpdateZeroIncrement => "WINDOW_UPDATE with zero increment",
            StreamError::WindowUpdateOverflow => "WINDOW_UPDATE caused window overflow",
            StreamError::InternalError => "ineternal error",
        };
        io::Error::new(io::ErrorKind::InvalidData, msg)
    }
}
