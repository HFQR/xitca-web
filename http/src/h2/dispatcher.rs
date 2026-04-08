use core::{
    cell::RefCell,
    cmp, fmt,
    future::poll_fn,
    mem,
    pin::{Pin, pin},
    task::{Poll, Waker},
};

use std::{
    collections::{HashMap, VecDeque},
    io,
    net::{Shutdown, SocketAddr},
    rc::Rc,
    time::Duration,
};

use tracing::error;
use xitca_io::{
    bytes::{Buf, BufMut, BytesMut},
    io::{AsyncBufRead, AsyncBufWrite, BoundedBuf, write_all},
};
use xitca_service::Service;
use xitca_unsafe_collection::{
    futures::{Select, SelectOutput},
    no_hash::NoHashBuilder,
};

use crate::{
    body::{Body, SizeHint},
    bytes::Bytes,
    config::HttpServiceConfig,
    date::{DateTime, DateTimeHandle},
    error::BodyError,
    http::{
        Extension, HeaderMap, Request, RequestExt, Response, Uri, Version,
        header::{CONTENT_LENGTH, DATE},
        uri,
    },
    util::{
        futures::Queue,
        timer::{KeepAlive, Timeout},
    },
};

use super::{
    body::RequestBody,
    proto::{
        PREFACE, data,
        error::Error,
        go_away::GoAway,
        head,
        headers::{self, ResponsePseudo},
        hpack,
        ping::Ping,
        reason::Reason,
        reset::Reset,
        settings::{self, Settings},
        stream_id::StreamId,
        window_update::WindowUpdate,
    },
};

struct DecodeContext<'a, S> {
    max_header_list_size: usize,
    max_concurrent_streams: usize,
    /// Highest stream ID fully accepted by the server. Sent in GOAWAY frames
    /// so the client knows which streams were processed (RFC 7540 §6.8).
    last_stream_id: StreamId,
    decoder: hpack::Decoder,
    next_frame_len: usize,
    continuation: Option<(headers::Headers, BytesMut)>,
    service: &'a S,
    ctx: &'a Shared,
    addr: SocketAddr,
    date: &'a DateTimeHandle,
}

type Frame = crate::body::Frame<Bytes>;

trait BodySize {
    /// Parse content-length from headers.
    /// Returns `Err(())` if the header is present but malformed (RFC 7540 §8.1.2.6).
    fn from_header(headers: &HeaderMap, is_end_stream: bool) -> Result<Self, ()>
    where
        Self: Sized;

    /// Subtract `len` bytes received in a DATA frame.
    /// Returns `Err(())` if this would underflow (overflow: more data than declared).
    fn dec(&mut self, len: usize) -> io::Result<()>;

    /// Returns `Err` if remaining != 0 at END_STREAM (underflow: less data than declared).
    fn ensure_zero(&self) -> io::Result<()>;
}

impl BodySize for SizeHint {
    fn from_header(headers: &HeaderMap, is_end_stream: bool) -> Result<Self, ()> {
        if is_end_stream {
            Ok(Self::None)
        } else {
            match headers.get(CONTENT_LENGTH) {
                Some(v) => {
                    let s = v.to_str().map_err(|_| ())?;
                    let len = headers::parse_u64(s.as_bytes())?;
                    Ok(Self::Exact(len))
                }
                None => Ok(Self::Unknown),
            }
        }
    }

    /// Subtract `len` bytes received in a DATA frame.
    /// Returns `Err(())` if this would underflow (overflow: more data than declared).
    fn dec(&mut self, len: usize) -> io::Result<()> {
        match self {
            Self::Unknown => Ok(()),
            Self::None => Err(io::Error::new(io::ErrorKind::InvalidData, "content-length exceeded")),
            Self::Exact(rem) => {
                *rem = rem
                    .checked_sub(len as u64)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "content-length exceeded"))?;
                Ok(())
            }
        }
    }

    /// Returns `Err` if remaining != 0 at END_STREAM (underflow: less data than declared).
    fn ensure_zero(&self) -> io::Result<()> {
        match self {
            Self::Unknown | Self::None | Self::Exact(0) => Ok(()),
            Self::Exact(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "content-length underflow")),
        }
    }
}

pub(super) struct StreamState {
    pub(super) recv_state: RecvState,
    send_state: SendState,
    /// Bitfield tracking closure state for both directions. See the associated
    /// constants below for the individual bit meanings.
    flag: u8,
}

impl StreamState {
    /// Stream is open in both receive and send direction
    const OPEN: u8 = 0;
    /// Peer sent END_STREAM or RST_STREAM: no more inbound DATA will arrive.
    /// `is_empty` uses this bit so the stream stays in the map until the peer
    /// is truly done, even if the service dropped `RequestBody` early.
    /// Also signals `poll_next` EOF on the graceful close path.
    const RECV_CLOSED: u8 = 1 << 0;
    /// Service dropped `RequestBody` without consuming it to EOF. The body
    /// consumer is gone but the peer may still be sending DATA frames that
    /// must be accounted for (content-length enforcement, window management).
    /// Also signals `poll_next` EOF on the early-cancel path.
    pub(super) const RECV_CANCELED: u8 = 1 << 1;
    /// Response task finished (half-closed local). Stream stays in the map
    /// until `RECV_CLOSED` is also set so WINDOW_UPDATE overflow can still
    /// be detected (RFC §6.9.1).
    const SEND_CLOSED: u8 = 1 << 2;

    #[allow(dead_code)]
    // TODO: strip response body for HEAD method request.
    const HEAD_METHOD: u8 = 1 << 3;

    fn recv_canceled(&self) -> bool {
        self.flag & Self::RECV_CANCELED == Self::RECV_CANCELED
    }

    pub(super) fn recv_closed(&self) -> bool {
        self.flag & Self::RECV_CLOSED == Self::RECV_CLOSED
    }

    fn send_closed(&self) -> bool {
        self.flag & Self::SEND_CLOSED == Self::SEND_CLOSED
    }

    pub(super) fn add_flag(&mut self, flag: u8) {
        self.flag |= flag;
    }

    fn closed(&self) -> bool {
        const CLOSED: u8 = StreamState::RECV_CLOSED | StreamState::SEND_CLOSED;
        self.flag & CLOSED == CLOSED
    }

    pub(super) fn is_empty(&self) -> bool {
        // Both directions must be closed at the protocol level before the
        // entry can be removed. RECV_CANCELED is a consumer-side signal and
        // does not gate removal.
        self.closed() && self.recv_state.queue.is_empty() && self.recv_state.error.is_none()
    }

    /// Push a frame to the recv queue, waking the body reader. No-op if the
    /// service already dropped `RequestBody` (RECV_CANCELED) or recv is closed.
    /// Returns `true` if the frame was actually enqueued.
    fn try_push_frame(&mut self, frame: Frame) -> bool {
        if !self.recv_closed() && !self.recv_canceled() {
            self.recv_state.queue.push_back(frame);
            self.recv_state.wake();
            true
        } else {
            false
        }
    }

    /// Set a recv-side error for the body reader. No-op if the service already
    /// dropped `RequestBody` (RECV_CANCELED) or the recv side is already closed.
    fn try_set_err(&mut self, err: io::Error) {
        if !self.recv_closed() && !self.recv_canceled() {
            self.recv_state.error = Some(Box::new(err));
        }
    }
}

pub(super) struct RecvState {
    /// Buffered DATA / Trailers frames for the request body, pushed by the
    /// decode path and drained by `RequestBody::poll_next`.
    pub(super) queue: VecDeque<Frame>,
    /// Waker stored by `RequestBody::poll_next` when the queue is empty;
    /// woken by the decode path after pushing new data.
    pub(super) waker: Option<Waker>,
    /// Remaining bytes the client may send on this stream (RFC 7540 §6.9).
    pub(super) window: usize,
    /// Terminal error set when the stream is reset by the peer (RST_STREAM)
    /// while `RequestBody` is still alive. Delivered as `Err` on the next
    /// `poll_next` call so the caller knows the body was truncated.
    pub(super) error: Option<BodyError>,
    /// RFC 7540 §8.1.2.6: tracks remaining expected DATA bytes.
    content_length: SizeHint,
}

impl RecvState {
    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

struct SendState {
    /// Remaining send window for this stream. Signed because a SETTINGS change
    /// reducing INITIAL_WINDOW_SIZE can drive it negative; the stream must not
    /// send until WINDOW_UPDATE brings it back above zero (RFC 7540 §6.9.2).
    window: i64,
    frame_size: usize,
    waker: Option<Waker>,
}

impl SendState {
    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

/// State machine for the server-initiated keepalive PING (CVE-2019-9512,
/// CVE-2019-9517).
///
/// - `Idle`     — no keepalive in flight; `PingPong::tick` may queue one.
/// - `Pending`  — `PingPong::tick` queued a PING; poll_encode has not encoded it yet.
/// - `InFlight` — poll_encode sent the PING; waiting for the peer's ACK.
///
/// `PingPong::tick` treats both `Pending` and `InFlight` as "not yet ACKed",
/// so it fires a timeout regardless of whether write_io is stalled (PING
/// never left) or the peer is silent (PING was sent but no ACK arrived).
#[derive(PartialEq)]
enum KeepalivePing {
    Idle,
    Pending,
    InFlight,
}

pub(super) struct FlowControl {
    /// Number of currently open streams (RFC 7540 §5.1.2).
    open_streams: usize,
    /// Remaining bytes we may send on the whole connection.
    send_connection_window: usize,
    /// Default send-window for new streams (RFC 7540 §6.9.2).
    stream_window: i64,
    /// Remote's SETTINGS_MAX_FRAME_SIZE.
    max_frame_size: usize,
    /// Remaining bytes we are willing to receive on the whole connection.
    recv_connection_window: usize,
    /// Per-stream state. Inserted when HEADERS arrives or response body starts;
    /// removed when both sides are done or on RST_STREAM.
    pub(super) stream_map: HashMap<StreamId, StreamState, NoHashBuilder>,
    /// The peer's latest SETTINGS frame, pending an ACK (CVE-2019-9515).
    /// A well-behaved peer sends one SETTINGS and waits for the ACK before
    /// sending another (RFC 7540 §6.5.3). A second SETTINGS arriving while
    /// one is already pending kills the connection with ENHANCE_YOUR_CALM.
    remote_settings: RemoteSettings,
    /// Net count of premature resets: incremented when RST_STREAM arrives for
    /// an active response task, decremented when a stream completes normally.
    /// Stays near zero for well-behaved clients; climbs toward
    /// PREMATURE_RESET_LIMIT only when resets consistently outpace completions.
    premature_reset_count: usize,
}

/// The peer's remote settings and their ACK state.
///
/// Stores the peer's latest non-ACK SETTINGS frame. `pending` is set on
/// receipt and cleared when the ACK is dispatched by `poll_encode`. The full
/// `Settings` value is kept so all relevant fields (e.g. HPACK table size)
/// can be applied atomically at ACK time.
#[derive(Default)]
struct RemoteSettings {
    settings: Settings,
    pending: bool,
}

impl RemoteSettings {
    /// Store incoming peer SETTINGS. Errors with ENHANCE_YOUR_CALM if a
    /// previous SETTINGS has not yet been ACKed (CVE-2019-9515).
    fn try_update(&mut self, settings: Settings) -> Result<(), Error> {
        if self.pending {
            return Err(Error::GoAway(Reason::ENHANCE_YOUR_CALM));
        }
        self.pending = true;
        self.settings = settings;
        Ok(())
    }

    /// Encode a SETTINGS ACK if one is pending. Applies the HPACK table size
    /// update before the ACK so the encoder is consistent with the wire order.
    /// Called at the top of every `poll_encode` pass ahead of header encoding.
    /// No-ops when nothing is pending.
    fn encode(&mut self, encoder: &mut hpack::Encoder, buf: &mut BytesMut) {
        if !self.pending {
            return;
        }
        if let Some(size) = self.settings.header_table_size() {
            encoder.update_max_size(size as usize);
        }
        Settings::ack().encode(buf);
        self.pending = false;
    }
}

impl FlowControl {
    /// Close both sides of a stream. Shortcut for calling `remove_recv` +
    /// `remove_send` together on unrecoverable protocol errors.
    fn remove_stream(&mut self, id: StreamId) {
        self.remove_recv(id);
        self.remove_send(id);
    }

    /// Close the receive side of a stream. Sets `RECV_CLOSED`, wakes any
    /// parked `RequestBody`, then removes the stream if both sides are done.
    fn remove_recv(&mut self, id: StreamId) {
        if let Some(state) = self.stream_map.get_mut(&id) {
            state.add_flag(StreamState::RECV_CLOSED);
            state.recv_state.wake();
            if state.is_empty() {
                self.stream_map.remove(&id);
            }
        }
    }

    /// Close the send side of a stream. Sets `SEND_CLOSED`, wakes any parked
    /// response task, then removes the stream if both sides are done.
    /// Returns `true` if the stream was present in the map (used by
    /// `StreamGuard::drop` to decide whether to credit back a premature reset).
    fn remove_send(&mut self, id: StreamId) -> bool {
        if let Some(state) = self.stream_map.get_mut(&id) {
            state.add_flag(StreamState::SEND_CLOSED);
            state.send_state.wake();
            if state.is_empty() {
                self.stream_map.remove(&id);
            }
            true
        } else {
            false
        }
    }

    /// Look up a stream by ID, distinguishing three cases:
    ///   - `Ok(Some(state))` — stream is active
    ///   - `Ok(None)`        — stream was previously open but is now closed; caller decides how to respond
    ///   - `Err`             — stream is idle (never opened); always a connection error PROTOCOL_ERROR
    fn get_stream_mut(&mut self, id: StreamId, last_stream_id: StreamId) -> Result<Option<&mut StreamState>, Error> {
        match self.stream_map.get_mut(&id) {
            Some(state) => Ok(Some(state)),
            None if id > last_stream_id => Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
            None => Ok(None),
        }
    }

    /// Apply `delta` to every active send stream's window and wake any that
    /// now have a positive window. Use `delta = 0` to wake without changing
    /// windows (e.g. after a connection-level WINDOW_UPDATE).
    fn update_and_wake_send_streams(&mut self, delta: i64) {
        for state in self.stream_map.values_mut() {
            let sf = &mut state.send_state;
            sf.window += delta;
            if sf.window > 0 {
                sf.wake();
            }
        }
    }

    // A RST_STREAM is "premature" if the stream is still tracked — either
    // the response task is running or the request body is still being
    // received. Count these to detect rapid-reset abuse (CVE-2023-44487).
    fn try_reset_stream(&mut self, id: StreamId) -> Result<(), Error> {
        let Some(state) = self.stream_map.get_mut(&id) else {
            return Ok(());
        };

        state.try_set_err(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "h2 stream reset by peer",
        ));

        self.premature_reset_count += 1;

        if self.premature_reset_count > PREMATURE_RESET_LIMIT {
            return Err(Error::GoAway(Reason::ENHANCE_YOUR_CALM));
        }

        self.remove_stream(id);

        Ok(())
    }
}

pub(super) enum Message {
    Head(headers::Headers<ResponsePseudo>),
    Data(data::Data),
    Trailer(headers::Headers<()>),
    Reset { stream_id: StreamId, reason: Reason },
    WindowUpdate { stream_id: StreamId, size: usize },
    GoAway { last_stream_id: StreamId, reason: Reason },
}

pub(super) struct WriterQueue {
    pub(super) messages: VecDeque<Message>,
    /// Bounded queue for RST_STREAM frames triggered by client-caused protocol
    /// errors (e.g. WINDOW_UPDATE incr=0, flow-control overflow). These are
    /// separated from `messages` so that a malicious client cannot exhaust
    /// heap memory by forcing the server to emit an unbounded number of resets.
    /// Server-initiated resets (service errors, body errors) stay in `messages`
    /// because they are bounded by `max_concurrent_streams`.
    resets: VecDeque<(StreamId, Reason)>,
    closed: bool,
    /// Accumulated connection-level WINDOW_UPDATE increment. Mutated at push
    /// time by all callers; flushed to a single connection frame after
    /// `poll_encode` drains the queue.
    pending_conn_window: usize,
    /// State of the server-initiated keepalive PING. Replaces the old
    /// `pending_ack` boolean; see `KeepalivePing` for the state transitions.
    keepalive_ping: KeepalivePing,
    /// A client-initiated PING whose ACK we must send. Stored outside the
    /// write queue so queue depth is permanently bounded (CVE-2019-9512).
    /// Always overwritten by the latest client PING; poll_encode takes and
    /// clears it after encoding the ACK.
    pending_client_ping: Option<[u8; 8]>,
}

/// All per-connection mutable state, shared between the read path, write path,
/// and any live `RequestBody` handles via a single `Rc<RefCell<…>>`.
pub(super) struct ConnectionInner {
    pub(super) flow: FlowControl,
    pub(super) queue: WriterQueue,
}

pub(super) type Shared = Rc<RefCell<ConnectionInner>>;

impl WriterQueue {
    fn new() -> Self {
        Self {
            messages: VecDeque::new(),
            resets: VecDeque::new(),
            closed: false,
            pending_conn_window: 0,
            keepalive_ping: KeepalivePing::Idle,
            pending_client_ping: None,
        }
    }

    fn push(&mut self, msg: Message) {
        self.messages.push_back(msg);
    }

    /// Accumulate a connection-level WINDOW_UPDATE into `pending_conn_window`.
    /// Flushed as a single connection frame after `poll_encode` drains the queue.
    pub(super) fn push_window_update(&mut self, size: usize) {
        self.pending_conn_window += size;
    }

    pub(super) fn push_data(&mut self, id: StreamId, payload: Bytes, end_stream: bool) {
        let mut data = data::Data::new(id, payload);
        data.set_end_stream(end_stream);
        self.push(Message::Data(data));
    }

    /// Push a RST_STREAM caused by a client protocol error. Returns `true` if
    /// the reset queue has reached `CLIENT_RESET_QUEUE_CAP`, indicating the
    /// caller should send GOAWAY and close the connection.
    fn push_client_reset(&mut self, stream_id: StreamId, reason: Reason) -> bool {
        self.resets.push_back((stream_id, reason));
        self.resets.len() > CLIENT_RESET_QUEUE_CAP
    }

    /// Set END_STREAM on the most recent DATA frame for `stream_id` in place,
    /// avoiding an extra zero-length frame in the common case (O(1)).
    ///
    /// On a single thread the body stream yields `None` immediately after its
    /// last chunk with no intervening yield point, so the tail of the queue IS
    /// the last DATA frame for this stream in the vast majority of cases.
    /// The O(n) search and zero-length fallback handle the rare exceptions.
    #[cold]
    #[inline(never)]
    fn push_end_stream(&mut self, stream_id: StreamId) {
        for msg in self.messages.iter_mut().rev() {
            if let Message::Data(d) = msg {
                if d.stream_id() == stream_id {
                    d.set_end_stream(true);
                    return;
                }
            }
        }

        // Fallback: last DATA already consumed by writer. Send a zero-length
        // DATA frame with END_STREAM (9 bytes on the wire, no window cost).
        self.push_end_stream_cold(stream_id);
    }

    #[cold]
    #[inline(never)]
    fn push_end_stream_cold(&mut self, stream_id: StreamId) {
        self.push_data(stream_id, Bytes::new(), true);
    }

    fn close(&mut self) {
        self.closed = true;
    }

    fn poll_recv(&mut self) -> Poll<Option<Message>> {
        if let Some((stream_id, reason)) = self.resets.pop_front() {
            return Poll::Ready(Some(Message::Reset { stream_id, reason }));
        }
        if let Some(msg) = self.messages.pop_front() {
            Poll::Ready(Some(msg))
        } else if self.closed {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

type Decoded = (Request<RequestExt<RequestBody>>, StreamId);

impl<'a, S> DecodeContext<'a, S> {
    fn check_not_idle(&self, id: StreamId) -> Result<(), Error> {
        if id > self.last_stream_id {
            Err(Error::GoAway(Reason::PROTOCOL_ERROR))
        } else {
            Ok(())
        }
    }

    /// Handle a stream-level error: queue RST_STREAM and replenish any
    /// connection window bytes consumed by the offending frame.
    /// Returns `Err(GoAway)` if the client has triggered too many resets.
    fn handle_stream_reset(&self, id: StreamId, reason: Reason, window: Option<usize>) -> Result<(), Error> {
        let mut inner = self.ctx.borrow_mut();
        let tx = &mut inner.queue;
        if tx.push_client_reset(id, reason) {
            return Err(Error::GoAway(Reason::ENHANCE_YOUR_CALM));
        }
        if let Some(size) = window {
            tx.push_window_update(size);
        }
        Ok(())
    }

    fn on_data_payload(&self, id: StreamId, payload: Bytes) -> Result<(), Error> {
        let payload_len = payload.len();
        let mut inner = self.ctx.borrow_mut();
        let flow = &mut inner.flow;

        if payload_len > flow.recv_connection_window {
            return Err(Error::GoAway(Reason::FLOW_CONTROL_ERROR));
        }
        flow.recv_connection_window -= payload_len;

        // Three cases (RFC 7540 §5.1 / §6.1):
        //   body_tx open  → active stream; forward payload, restore window on success.
        //   body_tx closed → half-closed (remote); DATA after END_STREAM → STREAM_CLOSED.
        //   Ok(None)       → fully closed stream → STREAM_CLOSED.
        // For the error cases the window is restored via the Reset handler in try_decode.
        let reason = match flow.get_stream_mut(id, self.last_stream_id)? {
            Some(state) if !state.recv_closed() => {
                // RFC 7540 §8.1.2.6: overflow — more data received than content-length declared.
                let reason = if let Err(err) = state.recv_state.content_length.dec(payload_len) {
                    state.try_set_err(err);
                    Reason::PROTOCOL_ERROR
                } else if payload_len > state.recv_state.window {
                    state.try_set_err(io::Error::new(io::ErrorKind::InvalidData, "flow control error"));
                    Reason::FLOW_CONTROL_ERROR
                } else {
                    state.recv_state.window -= payload_len;
                    // Service may have dropped RequestBody early (RECV_CANCELED).
                    // Still track content-length above, but discard the payload.
                    if !state.try_push_frame(Frame::Data(payload)) {
                        // Data discarded (RECV_CANCELED): replenish connection
                        // window so other streams don't stall.
                        inner.queue.push_window_update(payload_len);
                    }
                    return Ok(());
                };

                flow.remove_stream(id);

                reason
            }
            // RECV_CLOSED: half-closed (remote); any further DATA is STREAM_CLOSED.
            // stream not in map: fully closed; same error.
            _ => Reason::STREAM_CLOSED,
        };

        Err(Error::Reset(id, reason, Some(payload_len)))
    }

    fn new(
        ctx: &'a Shared,
        service: &'a S,
        max_concurrent_streams: usize,
        addr: SocketAddr,
        date: &'a DateTimeHandle,
    ) -> Self {
        Self {
            max_header_list_size: settings::DEFAULT_SETTINGS_MAX_HEADER_LIST_SIZE,
            max_concurrent_streams,
            last_stream_id: StreamId::zero(),
            decoder: hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE),
            next_frame_len: 0,
            continuation: None,
            ctx,
            service,
            addr,
            date,
        }
    }

    fn decode(&mut self, buf: &mut ReadBuf, mut on_msg: impl FnMut(&Self, Decoded)) -> Result<(), ShutDown> {
        let reason = loop {
            match self.try_decode(&mut buf.buf) {
                Ok(Some(res)) => on_msg(self, res),
                Ok(None) => return Ok(()),
                Err(Error::Reset(id, reason, window)) => match self.handle_stream_reset(id, reason, window) {
                    Ok(()) => {}
                    Err(Error::GoAway(reason)) => break reason,
                    Err(_) => unreachable!(),
                },
                Err(Error::GoAway(reason)) => break reason,
                Err(Error::Hpack(_)) => break Reason::COMPRESSION_ERROR,
                Err(Error::MalformedMessage) => break Reason::PROTOCOL_ERROR,
                Err(Error::PeerAccused) => {
                    self.ctx.borrow_mut().queue.close();
                    return Err(ShutDown::Forced);
                }
            }
        };

        Err(self.go_away(reason))
    }

    fn try_decode(&mut self, buf: &mut BytesMut) -> Result<Option<Decoded>, Error> {
        loop {
            if self.next_frame_len == 0 {
                if buf.len() < 3 {
                    return Ok(None);
                }
                let payload_len = buf.get_uint(3) as usize;
                if payload_len > settings::DEFAULT_MAX_FRAME_SIZE as usize {
                    return Err(Error::GoAway(Reason::FRAME_SIZE_ERROR));
                }
                self.next_frame_len = payload_len + 6;
            }

            if buf.len() < self.next_frame_len {
                return Ok(None);
            }

            let len = mem::replace(&mut self.next_frame_len, 0);
            let mut frame = buf.split_to(len);
            let head = head::Head::parse(&frame);

            // TODO: Make Head::parse auto advance the frame?
            frame.advance(6);

            if let Some(decoded) = self.decode_frame(head, frame)? {
                return Ok(Some(decoded));
            }
        }
    }

    /// Queue a GOAWAY frame with the given reason, close the write queue,
    /// and return the appropriate `ShutDown` variant.
    fn go_away(&self, reason: Reason) -> ShutDown {
        let mut inner = self.ctx.borrow_mut();
        inner.queue.push(Message::GoAway {
            last_stream_id: self.last_stream_id,
            reason,
        });
        inner.queue.close();
        if reason == Reason::NO_ERROR {
            ShutDown::Graceful
        } else {
            ShutDown::DrainWrite
        }
    }

    fn decode_frame(&mut self, head: head::Head, frame: BytesMut) -> Result<Option<Decoded>, Error> {
        match (head.kind(), &self.continuation) {
            (head::Kind::Continuation, _) => return self.handle_continuation(head, frame),
            // RFC 7540 §6.10: while a header block is in progress, the peer
            // MUST NOT send any frame type other than CONTINUATION.  Any
            // other frame on any stream is a connection error PROTOCOL_ERROR.
            (_, Some(_)) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
            (head::Kind::Headers, _) => {
                let (headers, payload) = headers::Headers::load(head, frame)?;
                let is_end_headers = headers.is_end_headers();
                return self.decode_header_block(headers, payload, is_end_headers);
            }
            (head::Kind::Data, _) => {
                let data = data::Data::load(head, frame.freeze())?;
                let is_end = data.is_end_stream();
                let id = data.stream_id();
                let payload = data.into_payload();

                self.check_not_idle(id)?;
                // A zero-length DATA frame without END_STREAM carries no
                // payload and signals nothing — it has no legitimate purpose.
                // The only valid use of empty DATA is to set END_STREAM on a
                // stream with no body (RFC 7540 §6.1).

                if payload.is_empty() && !is_end {
                    return Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None));
                }

                if !payload.is_empty() {
                    self.on_data_payload(id, payload)?;
                }

                if is_end {
                    let mut inner = self.ctx.borrow_mut();
                    let flow = &mut inner.flow;
                    if let Some(state) = flow.stream_map.get_mut(&id) {
                        // RFC 7540 §8.1.2.6: underflow — END_STREAM with bytes still expected.
                        if let Err(err) = state.recv_state.content_length.ensure_zero() {
                            state.try_set_err(err);
                            flow.remove_stream(id);
                            drop(inner);
                            return Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None));
                        }
                    }
                    flow.remove_recv(id);
                }
            }
            (head::Kind::WindowUpdate, _) => {
                let window = WindowUpdate::load(head, frame.as_ref())?;

                match (window.size_increment(), window.stream_id()) {
                    (0, StreamId::ZERO) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
                    (0, id) => {
                        self.check_not_idle(id)?;
                        self.ctx.borrow_mut().flow.remove_stream(id);
                        return Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None));
                    }
                    (incr, StreamId::ZERO) => {
                        let incr = incr as usize;
                        let mut inner = self.ctx.borrow_mut();
                        let flow = &mut inner.flow;

                        if flow.send_connection_window + incr > settings::MAX_INITIAL_WINDOW_SIZE {
                            return Err(Error::GoAway(Reason::FLOW_CONTROL_ERROR));
                        }

                        let was_zero = flow.send_connection_window == 0;
                        flow.send_connection_window += incr;

                        // Only wake streams if the connection window just became
                        // available — if it was already >0, no stream was blocked on it.
                        if was_zero {
                            flow.update_and_wake_send_streams(0);
                        }
                    }
                    (incr, id) => {
                        let incr = incr as i64;
                        let mut inner = self.ctx.borrow_mut();
                        let flow = &mut inner.flow;
                        let window = flow.send_connection_window;
                        if let Some(state) = flow.get_stream_mut(id, self.last_stream_id)? {
                            let sf = &mut state.send_state;
                            if sf.window + incr > settings::MAX_INITIAL_WINDOW_SIZE as i64 {
                                flow.remove_stream(id);
                                return Err(Error::Reset(id, Reason::FLOW_CONTROL_ERROR, None));
                            } else {
                                sf.window += incr;
                                if window > 0 {
                                    sf.wake();
                                }
                            }
                        }
                    }
                }
            }
            (head::Kind::Ping, _) => {
                let ping = Ping::load(head, frame.as_ref())?;
                if ping.is_ack {
                    // ACK for our keepalive PING: return to Idle so `PingPong::tick`
                    // does not time out on the next tick.
                    self.ctx.borrow_mut().queue.keepalive_ping = KeepalivePing::Idle;
                } else {
                    // Client-initiated PING: always overwrite; we reply with ACK.
                    self.ctx.borrow_mut().queue.pending_client_ping = Some(ping.payload);
                }
            }
            (head::Kind::Reset, _) => {
                let reset = Reset::load(head, frame.as_ref())?;
                let id = reset.stream_id();
                if id.is_zero() {
                    return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
                }
                self.check_not_idle(id)?;
                self.ctx.borrow_mut().flow.try_reset_stream(id)?;
            }
            (head::Kind::GoAway, _) => {
                // The peer is shutting down. Always respond with graceful
                // close regardless of the error code — RFC 7540 §7 says
                // unknown error codes MUST NOT trigger special behavior.
                let go_away = GoAway::load(head.stream_id(), frame.as_ref())?;
                if go_away.reason() != Reason::NO_ERROR {
                    tracing::warn!(
                        "received GOAWAY with error: {:?} last_stream={:?}",
                        go_away.reason(),
                        go_away.last_stream_id(),
                    );
                }
                return Err(Error::GoAway(Reason::NO_ERROR));
            }
            (head::Kind::Priority, _) => handle_priority(head.stream_id(), &frame)?,
            (head::Kind::PushPromise, _) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
            (head::Kind::Settings, _) => self.handle_settings(head, &frame)?,
            (head::Kind::Unknown, _) => {}
        }
        Ok(None)
    }

    #[cold]
    #[inline(never)]
    fn handle_continuation(&mut self, head: head::Head, frame: BytesMut) -> Result<Option<Decoded>, Error> {
        let is_end_headers = (head.flag() & 0x4) == 0x4;

        // RFC 7540 §6.10: CONTINUATION without a preceding incomplete HEADERS
        // is a connection error PROTOCOL_ERROR
        let (headers, mut payload) = self.continuation.take().ok_or(Error::GoAway(Reason::PROTOCOL_ERROR))?;

        if headers.stream_id() != head.stream_id() {
            return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
        }

        payload.extend_from_slice(&frame);
        self.decode_header_block(headers, payload, is_end_headers)
    }

    #[cold]
    #[inline(never)]
    fn handle_settings(&mut self, head: head::Head, frame: &BytesMut) -> Result<(), Error> {
        let setting = Settings::load(head, frame)?;

        if setting.is_ack() {
            return Ok(());
        }

        let mut inner = self.ctx.borrow_mut();
        let flow = &mut inner.flow;

        if let Some(new_window) = setting.initial_window_size() {
            let new_window = new_window as i64;
            let delta = new_window - flow.stream_window;
            flow.stream_window = new_window;

            if delta > 0 {
                let overflow = flow
                    .stream_map
                    .values()
                    .any(|s| s.send_state.window + delta > settings::MAX_INITIAL_WINDOW_SIZE as i64);
                if overflow {
                    return Err(Error::GoAway(Reason::FLOW_CONTROL_ERROR));
                }
            }

            if delta != 0 {
                flow.update_and_wake_send_streams(delta);
            }
        }

        if let Some(frame_size) = setting.max_frame_size() {
            let frame_size = frame_size as usize;
            flow.max_frame_size = frame_size;
            for state in flow.stream_map.values_mut() {
                state.send_state.frame_size = frame_size;
            }
        }

        // Record the pending ACK. A second SETTINGS before the first is ACKed
        // is a protocol violation and returns ENHANCE_YOUR_CALM (CVE-2019-9515).
        flow.remote_settings.try_update(setting)
    }

    fn decode_header_block(
        &mut self,
        mut headers: headers::Headers,
        mut payload: BytesMut,
        is_end_headers: bool,
    ) -> Result<Option<Decoded>, Error> {
        if let Err(e) = headers.load_hpack(&mut payload, self.max_header_list_size, &mut self.decoder) {
            return match e {
                // NeedMore on a multi-frame header block is normal; accumulate and wait
                // for CONTINUATION frames (RFC 7540 §6.10).
                Error::Hpack(hpack::DecoderError::NeedMore(_)) if !is_end_headers => {
                    self.continuation = Some((headers, payload));
                    Ok(None)
                }
                // Pseudo-header validation errors are stream-level (RFC 7540 §8.1.2).
                // The HPACK context was decoded successfully so no compression error.
                Error::MalformedMessage => {
                    let id = headers.stream_id();
                    self.last_stream_id = id;
                    Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None))
                }
                _ => Err(Error::GoAway(Reason::COMPRESSION_ERROR)),
            };
        }

        if !is_end_headers {
            self.continuation = Some((headers, payload));
            return Ok(None);
        }

        let id = headers.stream_id();
        self.handle_header_frame(id, headers)
    }

    fn handle_header_frame(&mut self, id: StreamId, headers: headers::Headers) -> Result<Option<Decoded>, Error> {
        let is_end_stream = headers.is_end_stream();

        let (pseudo, headers) = headers.into_parts();

        {
            let mut inner = self.ctx.borrow_mut();
            let flow = &mut inner.flow;
            if let Some(state) = flow.stream_map.get_mut(&id) {
                // RFC 7540 §8.1: trailer HEADERS MUST carry END_STREAM.
                if !is_end_stream {
                    flow.remove_recv(id);
                    return Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None));
                }
                if state.recv_closed() {
                    return Err(Error::Reset(id, Reason::STREAM_CLOSED, None));
                }
                // RFC 7540 §8.1.2.6: underflow — END_STREAM with bytes still expected.
                if let Err(err) = state.recv_state.content_length.ensure_zero() {
                    state.try_set_err(err);
                    flow.remove_stream(id);
                    return Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None));
                }
                state.try_push_frame(Frame::Trailers(headers));
                flow.remove_recv(id);
                return Ok(None);
            }
        }

        if !id.is_client_initiated() || id <= self.last_stream_id {
            return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
        }

        let Some(method) = pseudo.method else {
            // :method is required; stream was seen so last_stream_id must be updated.
            self.last_stream_id = id;
            return Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None));
        };

        let mut inner = self.ctx.borrow_mut();
        let flow = &mut inner.flow;

        // RFC 7540 §5.1.2: refuse new streams beyond the advertised limit.
        // Do NOT update last_stream_id: REFUSED_STREAM means no application
        // processing occurred and the client should retry (RFC 7540 §8.1.4).
        if flow.open_streams >= self.max_concurrent_streams {
            return Err(Error::Reset(id, Reason::REFUSED_STREAM, None));
        }

        // RFC 7540 §8.1.2.6: parse content-length before headers are moved into the request.
        // Only meaningful when there will be DATA frames (not end_stream).
        // Malformed content-length → RST_STREAM PROTOCOL_ERROR per §8.1.2.6.
        let content_length = BodySize::from_header(&headers, is_end_stream)
            .map_err(|_| Error::Reset(id, Reason::PROTOCOL_ERROR, None))?;

        // Initialize send_flow at stream creation so any WINDOW_UPDATE that
        // arrives before response_task starts its body loop is not lost.
        let sf = SendState {
            window: flow.stream_window,
            frame_size: flow.max_frame_size,
            waker: None,
        };

        self.last_stream_id = id;
        flow.open_streams += 1;

        let mut req = Request::new(RequestExt::from_parts((), Extension::new(self.addr)));
        *req.version_mut() = Version::HTTP_2;
        *req.headers_mut() = headers;
        *req.method_mut() = method;

        // TODO: make this fallible
        {
            let mut uri_parts = uri::Parts::default();

            if let Some(authority) = pseudo.authority {
                if let Ok(a) = uri::Authority::from_maybe_shared(authority.into_inner()) {
                    uri_parts.authority = Some(a);
                }
            }

            if let Some(scheme) = pseudo.scheme {
                if uri_parts.authority.is_some() {
                    if let Ok(s) = uri::Scheme::try_from(scheme.as_str()) {
                        uri_parts.scheme = Some(s);
                    }
                }
            }

            if let Some(path) = pseudo.path {
                if let Ok(pq) = uri::PathAndQuery::from_maybe_shared(path.into_inner()) {
                    uri_parts.path_and_query = Some(pq);
                }
            }

            if let Ok(uri) = Uri::from_parts(uri_parts) {
                *req.uri_mut() = uri;
            }
        }

        let body = RequestBody::new(id, content_length, Rc::clone(self.ctx));

        let flag = if is_end_stream {
            StreamState::RECV_CLOSED
        } else {
            StreamState::OPEN
        };

        flow.stream_map.insert(
            id,
            StreamState {
                recv_state: RecvState {
                    queue: VecDeque::new(),
                    waker: None,
                    window: if is_end_stream {
                        0
                    } else {
                        settings::DEFAULT_INITIAL_WINDOW_SIZE as usize
                    },
                    error: None,
                    content_length,
                },
                send_state: sf,
                flag,
            },
        );

        drop(inner);

        let req = req.map(|ext| ext.map_body(|_| body));

        Ok(Some((req, id)))
    }
}

/// Read buffer with a hard cap on accumulated unprocessed bytes.
///
/// `read_io` yields `Poll::Pending` without touching the socket when
/// `buf.len() >= cap`, preventing unbounded ingest when the write side is
/// stalled and frames cannot be drained fast enough.
struct ReadBuf {
    buf: BytesMut,
    cap: usize,
}

async fn read_io(mut rbuf: ReadBuf, io: &impl AsyncBufRead) -> (io::Result<usize>, ReadBuf) {
    if rbuf.buf.len() >= rbuf.cap {
        // Unprocessed data has hit the cap. Yield without issuing a new read
        // until the caller drains the buffer and restarts the task.
        return core::future::pending().await;
    }
    let len = rbuf.buf.len();
    rbuf.buf.reserve(4096);
    let (res, buf) = io.read(rbuf.buf.slice(len..)).await;
    (
        res,
        ReadBuf {
            buf: buf.into_inner(),
            cap: rbuf.cap,
        },
    )
}

async fn write_io(buf: BytesMut, io: &impl AsyncBufWrite) -> (io::Result<()>, BytesMut) {
    let (res, mut buf) = write_all(io, buf).await;
    buf.clear();
    (res, buf)
}

struct EncodeContext<'a> {
    encoder: hpack::Encoder,
    ctx: &'a Shared,
}

impl<'a> EncodeContext<'a> {
    fn new(ctx: &'a Shared) -> Self {
        Self {
            encoder: hpack::Encoder::new(65535, 4096),
            ctx,
        }
    }

    fn poll_encode(&mut self, write_buf: &mut BytesMut) -> Poll<bool> {
        let mut inner = self.ctx.borrow_mut();

        inner.flow.remote_settings.encode(&mut self.encoder, write_buf);

        let writable = loop {
            match inner.queue.poll_recv() {
                Poll::Ready(Some(msg)) => match msg {
                    Message::Head(headers) => {
                        let frame_size = inner.flow.max_frame_size;
                        let mut cont = headers.encode(&mut self.encoder, &mut write_buf.limit(frame_size));
                        while let Some(c) = cont {
                            cont = c.encode(&mut write_buf.limit(frame_size));
                        }
                    }
                    Message::Trailer(headers) => {
                        let frame_size = inner.flow.max_frame_size;
                        let mut cont = headers.encode(&mut self.encoder, &mut write_buf.limit(frame_size));
                        while let Some(c) = cont {
                            cont = c.encode(&mut write_buf.limit(frame_size));
                        }
                    }
                    Message::Data(mut data) => data.encode_chunk(write_buf),
                    Message::Reset { stream_id, reason } => Reset::new(stream_id, reason).encode(write_buf),
                    Message::WindowUpdate { stream_id, size } => {
                        WindowUpdate::new(stream_id, size as _).encode(write_buf)
                    }
                    Message::GoAway { last_stream_id, reason } => {
                        inner.queue.pending_conn_window = 0;
                        GoAway::new(last_stream_id, reason).encode(write_buf);
                        // Do NOT break with is_eof=true here. The graceful-drain
                        // path needs the write task to keep running after sending
                        // GOAWAY so that in-flight response frames (DATA, HEADERS,
                        // TRAILERS) queued by response tasks are still delivered.
                        // The write task exits naturally when the queue is both
                        // empty and closed (Poll::Ready(None) below).
                        break true;
                    }
                },
                Poll::Pending => break true,
                Poll::Ready(None) => break false,
            }
        };

        let pending = mem::replace(&mut inner.queue.pending_conn_window, 0);
        if pending > 0 {
            inner.flow.recv_connection_window += pending;
            WindowUpdate::new(StreamId::zero(), pending as _).encode(write_buf);
        }

        // Encode a client PING ACK if one is waiting (take-and-clear).
        if let Some(payload) = inner.queue.pending_client_ping.take() {
            head::Head::new(head::Kind::Ping, 0x1, StreamId::zero()).encode(8, write_buf);
            write_buf.put_slice(&payload);
        }

        // Encode our keepalive PING if it is queued but not yet sent, then
        // transition to InFlight so we do not re-send it on the next pass.
        if inner.queue.keepalive_ping == KeepalivePing::Pending {
            head::Head::new(head::Kind::Ping, 0x0, StreamId::zero()).encode(8, write_buf);
            write_buf.put_slice(&[0u8; 8]);
            inner.queue.keepalive_ping = KeepalivePing::InFlight;
        }

        if !write_buf.is_empty() {
            Poll::Ready(true)
        } else if !writable {
            Poll::Ready(false)
        } else {
            Poll::Pending
        }
    }
}

async fn response_task<S, ReqB, ResB, ResBE>(
    req: Request<RequestExt<RequestBody>>,
    stream_id: StreamId,
    service: &S,
    ctx: &Shared,
    date: &DateTimeHandle,
) where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ReqB: From<RequestBody>,
    ResB: Body<Data = Bytes, Error = ResBE>,
    ResBE: fmt::Debug,
{
    struct StreamGuard<'a> {
        stream_id: StreamId,
        ctx: &'a Shared,
    }

    impl Drop for StreamGuard<'_> {
        fn drop(&mut self) {
            let mut inner = self.ctx.borrow_mut();
            let flow = &mut inner.flow;
            flow.open_streams -= 1;
            // Stream was still in the map: it completed normally (not evicted
            // by RST_STREAM). Credit back one premature reset so well-behaved
            // connections never accumulate toward the limit over time.
            if flow.remove_send(self.stream_id) {
                flow.premature_reset_count = flow.premature_reset_count.saturating_sub(1);
            }
        }
    }

    let _guard = StreamGuard { stream_id, ctx };

    let req = req.map(|ext| ext.map_body(From::from));

    let res = match service.call(req).await {
        Ok(res) => res,
        Err(e) => {
            error!("service error: {:?}", e);
            ctx.borrow_mut().queue.push(Message::Reset {
                stream_id,
                reason: Reason::PROTOCOL_ERROR,
            });
            return;
        }
    };

    let (mut parts, body) = res.into_parts();

    let size = body.size_hint();

    if let SizeHint::Exact(size) = size {
        parts.headers.insert(CONTENT_LENGTH, size.into());
    }

    if !parts.headers.contains_key(DATE) {
        let date = date.with_date_header(Clone::clone);
        parts.headers.insert(DATE, date);
    }

    let pseudo = headers::Pseudo::response(parts.status);
    let mut headers = headers::Headers::new(stream_id, pseudo, parts.headers);

    let has_body = !matches!(size, SizeHint::None);
    if !has_body {
        headers.set_end_stream();
    }
    ctx.borrow_mut().queue.push(Message::Head(headers));

    if has_body {
        let mut body = pin!(body);

        'body: loop {
            match poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
                None => break ctx.borrow_mut().queue.push_end_stream(stream_id),
                Some(Err(e)) => {
                    error!("body error: {:?}", e);
                    ctx.borrow_mut().queue.push(Message::Reset {
                        stream_id,
                        reason: Reason::INTERNAL_ERROR,
                    });
                    break;
                }
                Some(Ok(Frame::Data(mut bytes))) => {
                    while !bytes.is_empty() {
                        let len = bytes.len();

                        let aval = poll_fn(|cx| {
                            let mut inner = ctx.borrow_mut();
                            let f = &mut inner.flow;

                            let Some(state) = f.stream_map.get_mut(&stream_id) else {
                                return Poll::Ready(None);
                            };
                            if state.send_closed() {
                                return Poll::Ready(None);
                            }

                            let sf = &mut state.send_state;
                            if f.send_connection_window == 0 || sf.window <= 0 {
                                sf.waker = Some(cx.waker().clone());
                                return Poll::Pending;
                            }

                            let len = cmp::min(len, sf.frame_size);
                            let aval = cmp::min(sf.window as usize, f.send_connection_window);
                            let aval = cmp::min(aval, len);
                            sf.window -= aval as i64;
                            f.send_connection_window -= aval;
                            Poll::Ready(Some(aval))
                        })
                        .await;

                        let Some(aval) = aval else {
                            break 'body;
                        };

                        let payload = bytes.split_to(aval);
                        let end_stream = bytes.is_empty() && body.is_end_stream();

                        ctx.borrow_mut().queue.push_data(stream_id, payload, end_stream);

                        if end_stream {
                            break 'body;
                        }
                    }
                }
                Some(Ok(Frame::Trailers(trailers))) => {
                    let trailer = headers::Headers::trailers(stream_id, trailers);
                    ctx.borrow_mut().queue.push(Message::Trailer(trailer));
                    break;
                }
            }
        }
    }
}

struct PingPong<'a> {
    timer: Pin<&'a mut KeepAlive>,
    ctx: &'a Shared,
    date: &'a DateTimeHandle,
    ka_dur: Duration,
}

impl<'a> PingPong<'a> {
    fn new(timer: Pin<&'a mut KeepAlive>, ctx: &'a Shared, date: &'a DateTimeHandle, ka_dur: Duration) -> Self {
        Self {
            timer,
            ctx,
            date,
            ka_dur,
        }
    }

    async fn tick(&mut self) -> io::Result<()> {
        self.timer.as_mut().await;
        // Keepalive tick: timeout if the previous PING was never ACKed
        // (write_io stalled or peer silent). Otherwise queue a new PING
        // and reset — write_task sees KeepalivePing::Pending on the next
        // select iteration with no explicit wake() required.
        {
            let mut inner = self.ctx.borrow_mut();

            if inner.queue.keepalive_ping != KeepalivePing::Idle {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "h2 ping timeout"));
            }
            inner.queue.keepalive_ping = KeepalivePing::Pending;
        }

        self.timer.as_mut().update(self.date.now() + self.ka_dur);

        Ok(())
    }
}

/// Maximum number of premature RST_STREAMs before the connection is closed
/// with GOAWAY(ENHANCE_YOUR_CALM). Matches the h2 crate's default.
const PREMATURE_RESET_LIMIT: usize = 100;
/// Maximum number of client-caused RST_STREAM frames that may queue up in
/// `WriterQueue::resets` at once. Exceeding this kills the connection with
/// GOAWAY(ENHANCE_YOUR_CALM). A well-behaved client should not trigger more
/// resets than it has open streams; a burst beyond this cap is abuse.
const CLIENT_RESET_QUEUE_CAP: usize = 64;
/// Hard cap on unprocessed bytes in the read buffer. `read_io` yields
/// `Poll::Pending` when this is reached, preventing unbounded ingest when
/// the write side cannot keep up.
const READ_BUF_CAP: usize = settings::DEFAULT_MAX_FRAME_SIZE as usize * 4; // 64 KiB

/// Peek into the given buffer (and read more if needed) to determine whether
/// the connection speaks HTTP/2.  Returns `(version, buf)` where `buf` contains
/// all bytes read so far (unconsumed) so the caller can forward them to the
/// chosen dispatcher.
pub(crate) async fn peek_version(
    io: &(impl AsyncBufRead + AsyncBufWrite),
    mut buf: BytesMut,
) -> io::Result<(crate::http::Version, BytesMut)> {
    use crate::http::Version;

    // We only need to see the first few bytes to distinguish h2 from h1.
    // The h2 client preface starts with "PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" (24 bytes).
    // Reading up to PREFACE.len() is enough for an unambiguous decision.
    while buf.len() < PREFACE.len() {
        let len = buf.len();
        buf.reserve(PREFACE.len() - len);
        let (res, b) = io.read(buf.slice(len..)).await;
        buf = b.into_inner();
        match res {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => return Err(e),
        }
    }

    let version = if buf.starts_with(PREFACE) {
        Version::HTTP_2
    } else {
        Version::HTTP_11
    };

    Ok((version, buf))
}

pub(crate) async fn run<
    Io,
    S,
    ReqB,
    ResB,
    ResBE,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
>(
    io: Io,
    addr: SocketAddr,
    read_buf: BytesMut,
    mut ka: Pin<&mut KeepAlive>,
    service: &S,
    date: &DateTimeHandle,
    config: &HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
) -> io::Result<()>
where
    Io: AsyncBufRead + AsyncBufWrite,
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    S::Error: fmt::Debug,
    ResB: Body<Data = Bytes, Error = ResBE>,
    ResBE: fmt::Debug,
{
    let mut settings = settings::Settings::default();

    settings.set_max_concurrent_streams(Some(config.h2_max_concurrent_streams));
    settings.set_initial_window_size(Some(config.h2_initial_window_size));
    settings.set_max_frame_size(Some(config.h2_max_frame_size));
    settings.set_max_header_list_size(Some(config.h2_max_header_list_size));
    settings.set_enable_connect_protocol(Some(1));

    let (buf, mut write_buf) = handshake(&io, read_buf, &settings, ka.as_mut()).await?;
    let mut read_buf = ReadBuf { buf, cap: READ_BUF_CAP };

    let recv_initial_window_size = settings
        .initial_window_size()
        .unwrap_or(settings::DEFAULT_INITIAL_WINDOW_SIZE);
    let max_frame_size = settings.max_frame_size().unwrap_or(settings::DEFAULT_MAX_FRAME_SIZE);
    let max_concurrent_streams = settings.max_concurrent_streams().unwrap_or(u32::MAX) as usize;

    let shared = Rc::new(RefCell::new(ConnectionInner {
        flow: FlowControl {
            open_streams: 0,
            // Send windows start at RFC 7540 §6.9.2 default (65535) until the
            // peer's SETTINGS_INITIAL_WINDOW_SIZE is received and applied.
            send_connection_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as usize,
            stream_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as i64,
            max_frame_size: max_frame_size as usize,
            recv_connection_window: recv_initial_window_size as usize,
            stream_map: HashMap::with_capacity_and_hasher(max_concurrent_streams, NoHashBuilder::default()),
            remote_settings: RemoteSettings::default(),
            premature_reset_count: 0,
        },
        queue: WriterQueue::new(),
    }));

    let mut ctx = DecodeContext::new(&shared, service, max_concurrent_streams, addr, date);
    let mut enc = EncodeContext::new(&shared);

    let mut queue = Queue::new();
    let mut ping_pong = PingPong::new(ka, &shared, date, config.keep_alive_timeout);

    let res = {
        let mut read_task = pin!(read_io(read_buf, &io));

        let mut write_task = pin!(async {
            while poll_fn(|_| enc.poll_encode(&mut write_buf)).await {
                let (res, buf) = io.write(write_buf).await;

                write_buf = buf;

                match res {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => write_buf.advance(n),
                    Err(e) => return Err(e),
                }
            }

            Ok(())
        });

        let shutdown = loop {
            match read_task
                .as_mut()
                .select(async {
                    loop {
                        let _ = queue.next().await;
                    }
                })
                .select(write_task.as_mut())
                .select(ping_pong.tick())
                .await
            {
                SelectOutput::A(SelectOutput::A(SelectOutput::A((res, buf)))) => {
                    read_buf = buf;

                    match res {
                        Ok(n) if n > 0 => {
                            if let Err(shutdown) = ctx.decode(&mut read_buf, |decoder, (req, id)| {
                                queue.push(response_task(req, id, decoder.service, decoder.ctx, decoder.date));
                            }) {
                                break shutdown;
                            }
                        }
                        res => break ShutDown::ReadClosed(res.map(|_| ())),
                    };

                    read_task.set(read_io(read_buf, &io));
                }
                SelectOutput::A(SelectOutput::A(SelectOutput::B(_))) => {}
                SelectOutput::A(SelectOutput::B(res)) => break ShutDown::WriteClosed(res),
                SelectOutput::B(Ok(_)) => {}
                SelectOutput::B(Err(e)) => break ShutDown::Timeout(e),
            }
        };

        Box::pin(async {
            let mut read_res = Ok(());

            match shutdown {
                ShutDown::WriteClosed(res) => return res,
                ShutDown::Timeout(err) => return Err(err),
                ShutDown::ReadClosed(res) => {
                    {
                        let mut inner = shared.borrow_mut();
                        for state in inner.flow.stream_map.values_mut() {
                            state.try_set_err(io::Error::new(
                                io::ErrorKind::ConnectionReset,
                                "h2 connection closed by peer",
                            ));
                            state.add_flag(StreamState::RECV_CLOSED);
                            state.recv_state.wake();
                        }
                        inner.queue.close();
                    }

                    read_res = res;
                }
                ShutDown::Graceful => {}
                ShutDown::DrainWrite => queue.clear(),
                ShutDown::Forced => return Ok(()),
            }

            loop {
                match queue.next().select(write_task.as_mut()).select(ping_pong.tick()).await {
                    SelectOutput::A(SelectOutput::A(_)) => {}
                    SelectOutput::A(SelectOutput::B(res)) => {
                        res?;
                        break read_res;
                    }
                    SelectOutput::B(res) => res?,
                }
            }
        })
        .await
    };

    // Send FIN so the peer sees a clean connection
    // close rather than RST (RFC 7540 §6.8).
    let _ = io.shutdown(Shutdown::Write).await;
    res
}

enum ShutDown {
    ReadClosed(io::Result<()>),
    Graceful,
    WriteClosed(io::Result<()>),
    Timeout(io::Error),
    DrainWrite,
    Forced,
}

/// Validate a PRIORITY frame payload (RFC 7540 §6.3, §5.3.1).
/// PRIORITY frames are deprecated in RFC 9113 and their content is ignored,
/// but the structural rules must still be enforced for RFC 7540 compatibility.
#[cold]
#[inline(never)]
fn handle_priority(id: StreamId, payload: &[u8]) -> Result<(), Error> {
    if id.is_zero() {
        Err(Error::GoAway(Reason::PROTOCOL_ERROR))
    } else if payload.len() != 5 {
        Err(Error::Reset(id, Reason::FRAME_SIZE_ERROR, None))
    } else if id == StreamId::parse(&payload[..4]).0 {
        Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None))
    } else {
        Ok(())
    }
}

type BoxedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Perform the HTTP/2 connection handshake: validate the client preface,
/// then send our SETTINGS frame. Returns the read buffer (with preface
/// consumed) and the write buffer (ready for reuse).
#[cold]
#[inline(never)]
fn handshake<'a>(
    io: &'a (impl AsyncBufRead + AsyncBufWrite),
    buf: BytesMut,
    settings: &'a settings::Settings,
    timer: Pin<&'a mut KeepAlive>,
) -> BoxedFuture<'a, io::Result<(BytesMut, BytesMut)>> {
    Box::pin(async {
        async {
            // No cap during preface: the buffer is tiny and always fully drained.
            let mut rbuf = ReadBuf { buf, cap: usize::MAX };
            while rbuf.buf.len() < PREFACE.len() {
                let (res, b) = read_io(rbuf, io).await;
                rbuf = b;
                res?;
            }
            if !rbuf.buf.starts_with(PREFACE) {
                // No GOAWAY is sent because our own preface has not been exchanged yet.
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid HTTP/2 client preface",
                ));
            }
            rbuf.buf.advance(PREFACE.len());

            let mut write_buf = BytesMut::new();
            settings.encode(&mut write_buf);
            let (res, write_buf) = write_io(write_buf, io).await;
            res?;

            Ok((rbuf.buf, write_buf))
        }
        .timeout(timer)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "h2 handshake timeout"))?
    })
}
