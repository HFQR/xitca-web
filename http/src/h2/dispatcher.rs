use core::{
    cell::RefCell,
    cmp, fmt,
    future::poll_fn,
    mem,
    pin::{Pin, pin},
    task::Poll,
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
    http::{
        Extension, HeaderMap, Method, Protocol, Request, RequestExt, Response, Uri, Version,
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
        error::Error,
        frame::{
            PREFACE, data,
            go_away::GoAway,
            head,
            headers::{self, ResponsePseudo},
            ping::Ping,
            reason::Reason,
            reset::Reset,
            settings::{self, Settings},
            stream_id::StreamId,
            window_update::WindowUpdate,
        },
        hpack,
        size::BodySize,
        stream::{RecvData, RecvStream, Stream},
    },
};

struct DecodeContext<'a, S> {
    max_header_list_size: usize,
    max_concurrent_streams: usize,
    decoder: hpack::Decoder,
    next_frame_len: usize,
    continuation: Option<(headers::Headers, BytesMut)>,
    service: &'a S,
    ctx: &'a Shared,
    addr: SocketAddr,
    date: &'a DateTimeHandle,
}

pub(super) type Frame = crate::body::Frame<Bytes>;
pub(super) type FrameBuffer = super::util::FrameBuffer<Frame>;

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

/// Tracks the highest stream id we've accepted from the peer plus whether
/// the boundary is still open to advancement.
///
/// - `Incrementable(id)` is the normal state. New HEADERS may advance the
///   boundary via `try_advance`.
/// - `Saturated(id)` is the post-GOAWAY state (RFC 7540 §6.8). The value
///   is frozen and serves as the boundary for silently dropping HEADERS
///   for higher stream ids; `try_advance` becomes a no-op.
///
/// All read paths (DATA / WINDOW_UPDATE / RST_STREAM / trailer routing)
/// use `get` and treat both variants identically — they only need the
/// value to detect frames addressed to idle stream ids (RFC 7540 §5.1).
#[derive(Clone, Copy)]
pub(super) enum LastStreamId {
    Incrementable(StreamId),
    Saturated(StreamId),
}

impl LastStreamId {
    /// Read the current boundary value regardless of variant.
    pub(super) fn get(self) -> StreamId {
        match self {
            Self::Incrementable(id) | Self::Saturated(id) => id,
        }
    }

    /// Returns `true` once `saturate` has been called. The dispatcher's
    /// main loop uses this to detect that the graceful drain may
    /// terminate as soon as the in-flight queue empties.
    pub(super) fn is_saturated(self) -> bool {
        matches!(self, Self::Saturated(_))
    }

    /// Advance the boundary to `id` if still incrementable. No-op once
    /// saturated, which is the mechanism by which post-GOAWAY HEADERS
    /// are dropped without bumping internal counters.
    pub(super) fn try_set(&mut self, id: StreamId) -> Result<Option<()>, Error> {
        match self {
            Self::Saturated(_) => Ok(None),
            Self::Incrementable(last_id) if !id.is_client_initiated() || id <= *last_id => {
                Err(Error::GoAway(Reason::PROTOCOL_ERROR))
            }
            Self::Incrementable(last_id) => {
                *last_id = id;
                Ok(Some(()))
            }
        }
    }

    /// try transit into Saturated state. Returns Some(last_stream_id) when succeeded
    pub(super) fn try_set_saturate(&mut self) -> Option<StreamId> {
        match *self {
            Self::Incrementable(id) => {
                let _ = mem::replace(self, LastStreamId::Saturated(id));
                Some(id)
            }
            _ => None,
        }
    }
}

pub(super) struct FlowControl {
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
    pub(super) stream_map: HashMap<StreamId, Stream, NoHashBuilder>,
    /// Net count of premature resets: incremented when RST_STREAM arrives for
    /// an active response task, decremented when a stream completes normally.
    /// Stays near zero for well-behaved clients; climbs toward
    /// PREMATURE_RESET_LIMIT only when resets consistently outpace completions.
    premature_reset_count: usize,
    /// Highest accepted client stream id, with explicit lifecycle: while
    /// `Incrementable` it advances on each new HEADERS; once GOAWAY is
    /// queued it transitions to `Saturated` and is frozen, which both
    /// causes new-stream HEADERS to be silently dropped (RFC 7540 §6.8)
    /// and signals the dispatcher's main loop that the graceful drain
    /// can terminate when the in-flight queue empties.
    last_stream_id: LastStreamId,
    pub(super) queue: WriterQueue,
    /// Shared slab backing all per-stream recv frame deques, avoiding
    /// per-stream `VecDeque` allocations.
    pub(super) frame_buf: FrameBuffer,
}

impl FlowControl {
    fn insert_stream(&mut self, id: StreamId, end_stream: bool, content_length: SizeHint) {
        let stream = Stream::new(self.stream_window, self.max_frame_size, content_length, end_stream);

        self.stream_map.insert(id, stream);
    }

    fn check_not_idle(&self, id: StreamId) -> Result<(), Error> {
        if id > self.last_stream_id.get() {
            Err(Error::GoAway(Reason::PROTOCOL_ERROR))
        } else {
            Ok(())
        }
    }

    fn recv_window_dec(&mut self, len: usize) -> Result<(), Error> {
        self.recv_connection_window = self
            .recv_connection_window
            .checked_sub(len)
            .ok_or(Error::GoAway(Reason::FLOW_CONTROL_ERROR))?;
        Ok(())
    }
    /// Try to remove a stream after END_STREAM is received.
    /// Only actually removes if both recv and send sides are closed.
    fn remove_stream(&mut self, id: StreamId) {
        if let Some(state) = self.stream_map.get_mut(&id) {
            if state.is_close() {
                self.stream_map.remove(&id);
            }
        }
    }

    /// Close the send side of a stream. Wakes any parked response task,
    /// then removes the stream if both sides are done.
    /// Returns `true` if the stream was present in the map (used by
    /// `StreamGuard::drop` to decide whether to credit back a premature reset).
    fn remove_send(&mut self, id: StreamId) -> bool {
        if let Some(state) = self.stream_map.get_mut(&id) {
            state.send.set_close();
            state.send.wake();
            if state.is_close() {
                self.stream_map.remove(&id);
            }
            true
        } else {
            false
        }
    }

    /// Apply `delta` to every active send stream's window and wake any that
    /// now have a positive window. Use `delta = 0` to wake without changing
    /// windows (e.g. after a connection-level WINDOW_UPDATE).
    fn update_and_wake_send_streams(&mut self, delta: i64) {
        for state in self.stream_map.values_mut() {
            let sf = &mut state.send;
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

        if state.is_recv_close() {
            // Body already dropped — close send side and remove entirely.
            state.send.set_close();
            state.send.wake();
            self.stream_map.remove(&id);
        } else {
            self.remove_send(id);
        }

        Ok(())
    }

    fn try_get_data_recv(&mut self, id: &StreamId) -> Result<RecvStream<'_>, Error> {
        let stream = self.stream_map.get_mut(id).ok_or(Error::Reset(Reason::STREAM_CLOSED))?;
        stream.try_get_recv(&mut self.frame_buf)
    }

    fn try_get_trailers_recv(&mut self, id: &StreamId) -> Result<RecvStream<'_>, Error> {
        let stream = self
            .stream_map
            .get_mut(id)
            .ok_or(Error::GoAway(Reason::STREAM_CLOSED))?;
        stream.try_get_recv(&mut self.frame_buf)
    }

    pub(super) fn request_body_drop(&mut self, id: &StreamId) {
        let stream = self
            .stream_map
            .get_mut(id)
            .expect("RequestBody is the owner of Recv type and stream MUST NOT be removed when it's still alive");

        stream.close_recv(&mut self.frame_buf);

        if stream.is_send_close() {
            self.stream_map.remove(id);
        }
    }

    fn handle_data(&mut self, id: StreamId, payload: Bytes, end_stream: bool) -> Result<(), Error> {
        let len = payload.len();
        self.recv_window_dec(len)?;
        self._handle_data(id, payload, end_stream)
            .inspect_err(|_| self.queue.push_window_update(len))
    }

    fn _handle_data(&mut self, id: StreamId, payload: Bytes, end_stream: bool) -> Result<(), Error> {
        let recv = self.try_get_data_recv(&id)?.try_recv_data(payload, end_stream);

        match recv {
            RecvData::Discard(size) | RecvData::StreamReset(size) => {
                // RequestBody dropped or stream is about to be reset. replenish connection window
                self.queue.push_window_update(size);

                // replenish stream window if not reset
                if matches!(recv, RecvData::Discard(_)) {
                    self.queue
                        .messages
                        .push_back(Message::WindowUpdate { stream_id: id, size });

                    // try remove stream when RequestBody is dropped
                    if end_stream {
                        self.remove_stream(id);
                    };
                }
            }
            RecvData::Queued => {}
        }

        Ok(())
    }
}

/// The peer's remote settings and their ACK state.
///
/// Stores the peer's latest non-ACK SETTINGS frame. `pending` is set on
/// receipt and cleared when the ACK is dispatched by `poll_encode`. The full
/// `Settings` value is kept so all relevant fields (e.g. HPACK table size)
/// can be applied atomically at ACK time.
#[derive(Default)]
struct RemoteSettings {
    header_table_size: Option<Option<u32>>,
}

impl RemoteSettings {
    /// Store incoming peer SETTINGS. Errors with ENHANCE_YOUR_CALM if a
    /// previous SETTINGS has not yet been ACKed (CVE-2019-9515).
    fn try_update(&mut self, settings: Settings) -> Result<(), Error> {
        if self.header_table_size.is_some() {
            return Err(Error::GoAway(Reason::ENHANCE_YOUR_CALM));
        }
        self.header_table_size = Some(settings.header_table_size());
        Ok(())
    }

    /// Encode a SETTINGS ACK if one is pending. Applies the HPACK table size
    /// update before the ACK so the encoder is consistent with the wire order.
    /// Called at the top of every `poll_encode` pass ahead of header encoding.
    /// No-ops when nothing is pending.
    fn encode(&mut self, encoder: &mut hpack::Encoder, buf: &mut BytesMut) {
        if let Some(header_table_size) = self.header_table_size.take() {
            if let Some(size) = header_table_size {
                encoder.update_max_size(size as usize);
            }
            Settings::ack().encode(buf);
        }
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
    /// The peer's latest SETTINGS frame, pending an ACK (CVE-2019-9515).
    /// A well-behaved peer sends one SETTINGS and waits for the ACK before
    /// sending another (RFC 7540 §6.5.3). A second SETTINGS arriving while
    /// one is already pending kills the connection with ENHANCE_YOUR_CALM.
    pending_settings: RemoteSettings,
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

pub(super) type Shared = Rc<RefCell<FlowControl>>;

impl WriterQueue {
    fn new() -> Self {
        Self {
            messages: VecDeque::new(),
            resets: VecDeque::new(),
            closed: false,
            pending_settings: RemoteSettings::default(),
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

    pub(super) fn push_trailers(&mut self, id: StreamId, trailers: HeaderMap) {
        let trailer = headers::Headers::trailers(id, trailers);
        self.push(Message::Trailer(trailer));
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
        self.push_data(stream_id, Bytes::new(), true);
    }

    fn close(&mut self) {
        self.closed = true;
    }

    fn poll_recv(&mut self) -> Poll<Option<Message>> {
        if let Some((stream_id, reason)) = self.resets.pop_front() {
            Poll::Ready(Some(Message::Reset { stream_id, reason }))
        } else if let Some(msg) = self.messages.pop_front() {
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
    /// Handle a stream-level error: queue RST_STREAM and replenish any
    /// connection window bytes consumed by the offending frame.
    /// Returns `Err(GoAway)` if the client has triggered too many resets.
    fn handle_stream_reset(&self, id: StreamId, reason: Reason) -> Result<(), Error> {
        let mut inner = self.ctx.borrow_mut();
        if inner.queue.push_client_reset(id, reason) {
            return Err(Error::GoAway(Reason::ENHANCE_YOUR_CALM));
        }
        Ok(())
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
            decoder: hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE),
            next_frame_len: 0,
            continuation: None,
            ctx,
            service,
            addr,
            date,
        }
    }

    fn decode(&mut self, buf: &mut BytesMut, mut on_msg: impl FnMut(&Self, Decoded)) -> Result<(), ShutDown> {
        let reason = loop {
            match self.try_decode(buf) {
                Ok(Some(res)) => on_msg(self, res),
                Ok(None) => return Ok(()),
                Err(Error::Reset(_)) => unreachable!(),
                Err(Error::GoAway(reason)) => break reason,
                Err(Error::Hpack(_)) => break Reason::COMPRESSION_ERROR,
                Err(Error::MalformedMessage) => break Reason::PROTOCOL_ERROR,
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

    /// Queue a GOAWAY frame with the given reason and close the write
    /// queue. Idempotent: subsequent calls (e.g. from a peer GOAWAY
    /// arriving after we already initiated shutdown, or from a critical
    /// error escalating an in-progress graceful drain) only update local
    /// state and do not emit a second GOAWAY frame on the wire.
    ///
    /// Always returns `ShutDown::DrainWrite`. Non-critical (`NO_ERROR`)
    /// shutdowns do not propagate through this path — see the GOAWAY
    /// frame handler in `decode_frame`, which calls this for side
    /// effects only and returns `Ok(None)` to keep the decode loop
    /// running until in-flight streams drain naturally.
    fn go_away(&self, reason: Reason) -> ShutDown {
        let mut inner = self.ctx.borrow_mut();
        if let Some(last_stream_id) = inner.last_stream_id.try_set_saturate() {
            inner.queue.push(Message::GoAway { last_stream_id, reason });
            inner.queue.close();
        }
        ShutDown::DrainWrite
    }

    fn decode_frame(&mut self, head: head::Head, frame: BytesMut) -> Result<Option<Decoded>, Error> {
        match self._decode_frame(head, frame) {
            Err(Error::Reset(reason)) => {
                self.handle_stream_reset(head.stream_id(), reason)?;
                Ok(None)
            }
            res => res,
        }
    }

    fn _decode_frame(&mut self, head: head::Head, frame: BytesMut) -> Result<Option<Decoded>, Error> {
        match (head.kind(), &self.continuation) {
            (head::Kind::Continuation, _) => return self.handle_continuation(head, frame),
            // RFC 7540 §6.10: while a header block is in progress, the peer
            // MUST NOT send any frame type other than CONTINUATION.  Any
            // other frame on any stream is a connection error PROTOCOL_ERROR.
            (_, Some(_)) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
            (head::Kind::Headers, _) => {
                let (headers, payload) = headers::Headers::load(head, frame)?;
                let is_end_headers = headers.is_end_headers();
                return self.handle_headers(headers, payload, is_end_headers);
            }
            (head::Kind::Data, _) => {
                let data = data::Data::load(head, frame.freeze())?;
                let is_end = data.is_end_stream();
                let id = data.stream_id();
                let payload = data.into_payload();

                let mut state = self.ctx.borrow_mut();
                state.check_not_idle(id)?;
                state.handle_data(id, payload, is_end)?;
            }
            (head::Kind::WindowUpdate, _) => {
                let window = WindowUpdate::load(head, frame.as_ref())?;

                let flow = &mut self.ctx.borrow_mut();

                let id = window.stream_id();
                flow.check_not_idle(id)?;

                match (window.size_increment(), id) {
                    (0, StreamId::ZERO) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
                    (0, id) => {
                        if let Some(state) = flow.stream_map.get_mut(&id) {
                            state.try_set_err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "WINDOW_UPDATE with zero increment",
                            ));
                        }
                        return Err(Error::Reset(Reason::PROTOCOL_ERROR));
                    }
                    (incr, StreamId::ZERO) => {
                        let incr = incr as usize;

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
                        let window = flow.send_connection_window;
                        if let Some(state) = flow.stream_map.get_mut(&id) {
                            let sf = &mut state.send;
                            if sf.window + incr > settings::MAX_INITIAL_WINDOW_SIZE as i64 {
                                state.try_set_err(io::Error::new(io::ErrorKind::InvalidData, "WINDOW_UPDATE overflow"));
                                return Err(Error::Reset(Reason::FLOW_CONTROL_ERROR));
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
                let mut inner = self.ctx.borrow_mut();
                inner.check_not_idle(id)?;
                inner.try_reset_stream(id)?;
            }
            (head::Kind::GoAway, _) => {
                // The peer is shutting down. Always treat it as a graceful
                // shutdown regardless of the error code — RFC 7540 §7 says
                // unknown error codes MUST NOT trigger special behavior.
                let go_away = GoAway::load(head.stream_id(), frame.as_ref())?;
                if go_away.reason() != Reason::NO_ERROR {
                    tracing::warn!(
                        "received GOAWAY with error: {:?} last_stream={:?}",
                        go_away.reason(),
                        go_away.last_stream_id(),
                    );
                }
                // Mirror the peer's intent: queue our own GOAWAY and mark
                // `flow.goaway_sent` so the dispatcher's main loop can
                // terminate the graceful drain once all in-flight streams
                // complete. RFC 7540 §6.8 explicitly allows the peer to
                // keep sending frames (DATA, WINDOW_UPDATE, RST_STREAM,
                // trailers) for streams that were already in flight at
                // the time the GOAWAY was sent, so we deliberately do not
                // bail out of the decode loop — subsequent frames in the
                // same buffer must still be processed.
                self.go_away(Reason::NO_ERROR);
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

        let (headers, mut payload) = self.continuation.take().ok_or(Error::GoAway(Reason::PROTOCOL_ERROR))?;

        // RFC 7540 §6.10: CONTINUATION without a preceding incomplete HEADERS
        // is a connection error PROTOCOL_ERROR
        if headers.stream_id() != head.stream_id() {
            return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
        }

        payload.extend_from_slice(&frame);
        self.handle_headers(headers, payload, is_end_headers)
    }

    #[cold]
    #[inline(never)]
    fn handle_settings(&mut self, head: head::Head, frame: &BytesMut) -> Result<(), Error> {
        let setting = Settings::load(head, frame)?;

        if setting.is_ack() {
            return Ok(());
        }

        let mut flow = self.ctx.borrow_mut();

        if let Some(new_window) = setting.initial_window_size() {
            let new_window = new_window as i64;
            let delta = new_window - flow.stream_window;
            flow.stream_window = new_window;

            if delta > 0 {
                let overflow = flow
                    .stream_map
                    .values()
                    .any(|s| s.send.window + delta > settings::MAX_INITIAL_WINDOW_SIZE as i64);
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
                state.send.frame_size = frame_size;
            }
        }

        // Record the pending ACK. A second SETTINGS before the first is ACKed
        // is a protocol violation and returns ENHANCE_YOUR_CALM (CVE-2019-9515).
        flow.queue.pending_settings.try_update(setting)
    }

    fn handle_headers(
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
                    if self.ctx.borrow_mut().last_stream_id.try_set(id)?.is_none() {
                        return Ok(None);
                    }
                    Err(Error::Reset(Reason::PROTOCOL_ERROR))
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
        let end_stream = headers.is_end_stream();

        let (pseudo, headers) = headers.into_parts();

        let mut flow = self.ctx.borrow_mut();
        if flow.last_stream_id.get() >= id {
            flow.try_get_trailers_recv(&id)?.try_recv_trailers(headers, end_stream);

            if end_stream {
                flow.remove_stream(id);
            }
            return Ok(None);
        }

        // Validate and advance the stream-ID boundary before any
        // application-level checks. This ensures protocol violations
        // (non-client-initiated, non-monotonic) produce GOAWAY rather
        // than being masked by REFUSED_STREAM.
        // Saturated (post-GOAWAY): silently drop the frame.
        if flow.last_stream_id.try_set(id)?.is_none() {
            return Ok(None);
        }

        // RFC 7540 §5.1.2: refuse new streams beyond the advertised limit.
        // stream_map holds streams in open / half-closed states — exactly
        // those that count toward the concurrent-stream limit.
        if flow.stream_map.len() >= self.max_concurrent_streams {
            return Err(Error::Reset(Reason::REFUSED_STREAM));
        }

        // RFC 7540 §8.1.2.6: parse content-length before headers are moved into the request.
        // Only meaningful when there will be DATA frames (not end_stream).
        // Malformed content-length → RST_STREAM PROTOCOL_ERROR per §8.1.2.6.
        let content_length =
            BodySize::from_header(&headers, end_stream).map_err(|_| Error::Reset(Reason::PROTOCOL_ERROR))?;

        // :method is required; stream was seen so the boundary must
        // advance (no-op once saturated, but the saturation gate
        // above already returned in that case).
        let method = pseudo.method.ok_or(Error::Reset(Reason::PROTOCOL_ERROR))?;

        let protocol = pseudo.protocol.map(|proto| Protocol::from_str(&proto));

        // RFC 8441 §4: extended CONNECT follows normal request rules.
        let is_strict_connect = method == Method::CONNECT && protocol.is_none();

        // Validate and build URI from pseudo-headers in one pass.
        // RFC 7540 §8.3: regular CONNECT MUST NOT include :scheme or :path.
        // RFC 7540 §8.1.2.3: non-CONNECT MUST include :scheme and non-empty :path.
        let mut uri_parts = uri::Parts::default();

        if let Some(authority) = pseudo.authority {
            if let Ok(a) = uri::Authority::from_maybe_shared(authority.into_inner()) {
                uri_parts.authority = Some(a);
            }
        }

        match (is_strict_connect, pseudo.scheme) {
            // RFC 7540 §8.1.2.3: non-CONNECT MUST include :scheme.
            // RFC 7540 §8.3: regular CONNECT MUST NOT include :scheme.
            (true, Some(_)) | (false, None) => return Err(Error::Reset(Reason::PROTOCOL_ERROR)),
            (false, Some(scheme)) if uri_parts.authority.is_some() => {
                if let Ok(s) = uri::Scheme::try_from(scheme.as_str()) {
                    uri_parts.scheme = Some(s);
                }
            }
            _ => {}
        }

        match (is_strict_connect, pseudo.path) {
            // RFC 7540 §8.1.2.3: non-CONNECT MUST include :path.
            // RFC 7540 §8.3: regular CONNECT MUST NOT include :path.
            (true, Some(_)) | (false, None) => return Err(Error::Reset(Reason::PROTOCOL_ERROR)),
            (_, Some(path)) if !path.is_empty() => {
                if let Ok(pq) = uri::PathAndQuery::from_maybe_shared(path.into_inner()) {
                    uri_parts.path_and_query = Some(pq);
                }
            }
            // RFC 7540 §8.1.2.3: non-CONNECT MUST include non empty :path.
            (false, _) => return Err(Error::Reset(Reason::PROTOCOL_ERROR)), // empty :path
            _ => {}
        }

        let ext = Extension::with_protocol(self.addr, protocol);

        let mut req = Request::new(RequestExt::from_parts((), ext));
        *req.version_mut() = Version::HTTP_2;
        *req.headers_mut() = headers;
        *req.method_mut() = method;

        if let Ok(uri) = Uri::from_parts(uri_parts) {
            *req.uri_mut() = uri;
        }

        let body = RequestBody::new(id, content_length, Rc::clone(self.ctx));

        flow.insert_stream(id, end_stream, content_length);

        let req = req.map(|ext| ext.map_body(|_| body));

        Ok(Some((req, id)))
    }
}

async fn read_io<const LIMIT: usize>(mut buf: BytesMut, io: &impl AsyncBufRead) -> (io::Result<usize>, BytesMut) {
    if buf.len() >= LIMIT {
        // Unprocessed data has hit the cap. Yield without issuing a new read
        // until the caller drains the buffer and restarts the task.
        return core::future::pending().await;
    }
    let len = buf.len();
    buf.reserve(4096);
    let (res, buf) = io.read(buf.slice(len..)).await;
    (res, buf.into_inner())
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
        let mut flow = self.ctx.borrow_mut();

        // remote_setting can contain updated states for following encoding
        flow.queue.pending_settings.encode(&mut self.encoder, write_buf);

        let writable = loop {
            match flow.queue.poll_recv() {
                Poll::Ready(Some(msg)) => match msg {
                    Message::Head(headers) => {
                        let frame_size = flow.max_frame_size;
                        let mut cont = headers.encode(&mut self.encoder, &mut write_buf.limit(frame_size));
                        while let Some(c) = cont {
                            cont = c.encode(&mut write_buf.limit(frame_size));
                        }
                    }
                    Message::Trailer(headers) => {
                        let frame_size = flow.max_frame_size;
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
                        flow.queue.pending_conn_window = 0;
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

        let pending = mem::replace(&mut flow.queue.pending_conn_window, 0);
        if pending > 0 {
            flow.recv_connection_window += pending;
            WindowUpdate::new(StreamId::zero(), pending as _).encode(write_buf);
        }

        // Encode a client PING ACK if one is waiting (take-and-clear).
        if let Some(payload) = flow.queue.pending_client_ping.take() {
            head::Head::new(head::Kind::Ping, 0x1, StreamId::zero()).encode(8, write_buf);
            write_buf.put_slice(&payload);
        }

        // Encode our keepalive PING if it is queued but not yet sent, then
        // transition to InFlight so we do not re-send it on the next pass.
        if flow.queue.keepalive_ping == KeepalivePing::Pending {
            head::Head::new(head::Kind::Ping, 0x0, StreamId::zero()).encode(8, write_buf);
            write_buf.put_slice(&[0u8; 8]);
            flow.queue.keepalive_ping = KeepalivePing::InFlight;
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
    let guard = StreamGuard { stream_id, ctx };

    let req = req.map(|ext| ext.map_body(From::from));

    let res = match service.call(req).await {
        Ok(res) => res,
        Err(e) => {
            error!("service error: {:?}", e);
            // Only send RST_STREAM if the stream is still active. If the peer
            // already reset it (or the dispatcher removed it for another reason),
            // the error may have originated from try_set_err and sending a
            // redundant reset is unnecessary.
            let mut flow = ctx.borrow_mut();
            if flow.stream_map.contains_key(&stream_id) {
                flow.queue.push(Message::Reset {
                    stream_id,
                    reason: Reason::PROTOCOL_ERROR,
                });
            }
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
                None => break ctx.borrow_mut().queue.push_end_stream(guard.stream_id),
                Some(Err(e)) => {
                    error!("body error: {:?}", e);
                    break guard.ctx.borrow_mut().queue.push(Message::Reset {
                        stream_id: guard.stream_id,
                        reason: Reason::INTERNAL_ERROR,
                    });
                }
                Some(Ok(Frame::Data(bytes))) => {
                    if guard.send_data(bytes, body.is_end_stream()).await {
                        break 'body;
                    }
                }
                Some(Ok(Frame::Trailers(trailers))) => break guard.send_trailers(trailers),
            }
        }
    }
}

struct StreamGuard<'a> {
    stream_id: StreamId,
    ctx: &'a Shared,
}

impl Drop for StreamGuard<'_> {
    fn drop(&mut self) {
        let mut flow = self.ctx.borrow_mut();
        // Stream was still in the map: it completed normally (not evicted
        // by RST_STREAM). Credit back one premature reset so well-behaved
        // connections never accumulate toward the limit over time.
        if flow.remove_send(self.stream_id) {
            flow.premature_reset_count = flow.premature_reset_count.saturating_sub(1);
        }
    }
}

impl StreamGuard<'_> {
    // return true to inform outer loop to break
    async fn send_data(&self, mut data: Bytes, end_stream: bool) -> bool {
        if data.is_empty() && !end_stream {
            tracing::warn!("response body should not yield empty Frame::Data unless it's the last chunk of Body");
            return false;
        }

        loop {
            let len = data.len();

            let opt = poll_fn(|cx| {
                let mut flow = self.ctx.borrow_mut();
                let send_connection_window = flow.send_connection_window;
                let Some(state) = flow.stream_map.get_mut(&self.stream_id) else {
                    return Poll::Ready(None);
                };

                if state.is_send_close() {
                    return Poll::Ready(None);
                }

                let sf = &mut state.send;
                if len > 0 && (send_connection_window == 0 || sf.window <= 0) {
                    sf.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }

                let len = cmp::min(len, sf.frame_size);
                let aval = cmp::min(sf.window as usize, send_connection_window);
                let aval = cmp::min(aval, len);
                sf.window -= aval as i64;
                flow.send_connection_window -= aval;
                Poll::Ready(Some((aval, flow)))
            })
            .await;

            let Some((aval, mut flow)) = opt else {
                return true;
            };

            let all_consumed = aval == len;

            let payload = if all_consumed {
                mem::take(&mut data)
            } else {
                data.split_to(aval)
            };

            let end_stream = all_consumed && end_stream;

            flow.queue.push_data(self.stream_id, payload, end_stream);

            if end_stream {
                return true;
            } else if all_consumed {
                return false;
            }
        }
    }

    fn send_trailers(&self, trailers: HeaderMap) {
        self.ctx.borrow_mut().queue.push_trailers(self.stream_id, trailers);
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

/// Peek into the given buffer (and read more if needed) to determine whether
/// the connection speaks HTTP/2.  Returns `(version, buf)` where `buf` contains
/// all bytes read so far (unconsumed) so the caller can forward them to the
/// chosen dispatcher.
pub(crate) async fn peek_version(
    io: &(impl AsyncBufRead + AsyncBufWrite),
    buf: BytesMut,
) -> io::Result<(Version, BytesMut)> {
    let (read_buf, res) = prefix_check::<4096>(buf, io).await;
    let version = if res.is_ok() { Version::HTTP_2 } else { Version::HTTP_11 };
    Ok((version, read_buf))
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

    let (mut read_buf, mut write_buf) = handshake::<READ_BUF_LIMIT>(&io, read_buf, &settings, ka.as_mut()).await?;

    let recv_initial_window_size = settings
        .initial_window_size()
        .unwrap_or(settings::DEFAULT_INITIAL_WINDOW_SIZE);
    let max_frame_size = settings.max_frame_size().unwrap_or(settings::DEFAULT_MAX_FRAME_SIZE);
    let max_concurrent_streams = settings.max_concurrent_streams().unwrap_or(u32::MAX) as usize;

    let shared = Rc::new(RefCell::new(FlowControl {
        // Send windows start at RFC 7540 §6.9.2 default (65535) until the
        // peer's SETTINGS_INITIAL_WINDOW_SIZE is received and applied.
        send_connection_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as usize,
        stream_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as i64,
        max_frame_size: max_frame_size as usize,
        recv_connection_window: recv_initial_window_size as usize,
        stream_map: HashMap::with_capacity_and_hasher(max_concurrent_streams, NoHashBuilder::default()),
        premature_reset_count: 0,
        last_stream_id: LastStreamId::Incrementable(StreamId::ZERO),
        queue: WriterQueue::new(),
        frame_buf: FrameBuffer::new(),
    }));

    let mut ctx = DecodeContext::new(&shared, service, max_concurrent_streams, addr, date);
    let mut enc = EncodeContext::new(&shared);

    let mut queue = Queue::new();
    let mut ping_pong = PingPong::new(ka, &shared, date, config.keep_alive_timeout);

    let res = {
        let mut read_task = pin!(read_io::<READ_BUF_LIMIT>(read_buf, &io));

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
                        // Once a GOAWAY has been queued (either initiated
                        // locally or mirrored from the peer at L955), the
                        // graceful drain completes the moment there are
                        // no more in-flight response tasks. Returning here
                        // breaks the main loop into the write-only drain
                        // path with empty stream/queue state, so the
                        // subsequent fault/clear is a no-op.
                        if queue.is_empty() && shared.borrow().last_stream_id.is_saturated() {
                            return;
                        }
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
                // The inner future returns only when the graceful drain is
                // complete (queue empty and a GOAWAY has been sent). Falls
                // through to the write-only drain path with empty state.
                SelectOutput::A(SelectOutput::A(SelectOutput::B(_))) => break ShutDown::DrainWrite,
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
                        let mut flow = shared.borrow_mut();
                        for state in flow.stream_map.values_mut() {
                            // Open → Error + wake; Close stays Close.
                            // Return value ignored: connection-level teardown,
                            // streams drain via StreamGuard/RequestBody drops.
                            state.try_set_err(io::Error::new(
                                io::ErrorKind::ConnectionReset,
                                "h2 connection closed by peer",
                            ));
                            // No more data will arrive on any stream.
                            // Close recv; Error left intact for delivery.
                            state.recv.set_close_2();
                        }
                        flow.queue.close();
                    }

                    read_res = res;
                }
                ShutDown::DrainWrite => queue.clear(),
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
    WriteClosed(io::Result<()>),
    Timeout(io::Error),
    DrainWrite,
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
        Err(Error::Reset(Reason::FRAME_SIZE_ERROR))
    } else if id == StreamId::parse(&payload[..4]).0 {
        Err(Error::Reset(Reason::PROTOCOL_ERROR))
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
fn handshake<'a, const LIMIT: usize>(
    io: &'a (impl AsyncBufRead + AsyncBufWrite),
    buf: BytesMut,
    settings: &'a settings::Settings,
    timer: Pin<&'a mut KeepAlive>,
) -> BoxedFuture<'a, io::Result<(BytesMut, BytesMut)>> {
    Box::pin(async {
        async {
            // No cap during preface: the buffer is tiny and always fully drained.
            let (mut read_buf, res) = prefix_check::<LIMIT>(buf, io).await;
            res?;
            read_buf.advance(PREFACE.len());

            let mut write_buf = BytesMut::new();
            settings.encode(&mut write_buf);
            let (res, write_buf) = write_io(write_buf, io).await;
            res?;

            Ok((read_buf, write_buf))
        }
        .timeout(timer)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "h2 handshake timeout"))?
    })
}

// only check the prefix but not consume it
async fn prefix_check<const LIMIT: usize>(
    mut read_buf: BytesMut,
    io: &(impl AsyncBufRead + AsyncBufWrite),
) -> (BytesMut, io::Result<()>) {
    while read_buf.len() < PREFACE.len() {
        let (res, b) = read_io::<LIMIT>(read_buf, io).await;
        read_buf = b;

        if res.is_err() {
            return (read_buf, res.map(|_| ()));
        };
    }

    let res = if !read_buf.starts_with(PREFACE) {
        // No GOAWAY is sent because our own preface has not been exchanged yet.
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP/2 client preface",
        ))
    } else {
        Ok(())
    };

    (read_buf, res)
}
