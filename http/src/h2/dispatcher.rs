use core::{
    cell::RefCell,
    fmt,
    future::{Future, poll_fn},
    mem,
    pin::{Pin, pin},
    task::{Context, Poll, ready},
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
    STREAM_MUST_EXIST,
    body::RequestBody,
    proto::{
        error::Error,
        frame::{
            PREFACE,
            data::Data,
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
        last_stream_id::LastStreamId,
        ping_pong::PingPong,
        reset_counter::ResetCounter,
        size::BodySize,
        stream::{RecvClose, RecvData, Remove, Stream, StreamError},
        threshold::StreamRecvWindowThreshold,
    },
};

struct DecodeContext<'a, S> {
    max_header_list_size: usize,
    max_concurrent_streams: usize,
    recv_threshold: StreamRecvWindowThreshold,
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

pub(super) struct FlowControl {
    /// Remaining bytes we may send on the whole connection.
    send_connection_window: usize,
    /// Default send-window for new streams (RFC 7540 §6.9.2).
    send_stream_initial_window: i64,
    /// Remote's SETTINGS_MAX_FRAME_SIZE.
    max_frame_size: usize,
    /// Default recv-window for new streams
    recv_stream_initial_window: usize,
    /// Remaining bytes we are willing to receive on the whole connection.
    recv_connection_window: usize,
    /// Per-stream state. Inserted when HEADERS arrives or response body starts;
    /// removed when both sides are done or on RST_STREAM.
    pub(super) stream_map: HashMap<StreamId, Stream, NoHashBuilder>,
    /// Sliding-window counter for client-caused resets (CVE-2023-44487).
    /// Counts both peer-initiated RST_STREAM and server-generated RST_STREAM
    /// in response to client protocol errors within a time window.
    reset_counter: ResetCounter,
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
        let stream = Stream::new(
            self.send_stream_initial_window,
            self.max_frame_size,
            self.recv_stream_initial_window,
            content_length,
            end_stream,
        );

        self.stream_map.insert(id, stream);
    }

    fn check_not_idle(&self, id: StreamId) -> Result<(), Error> {
        if self.last_stream_id.check_idle(id) {
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
        let Some(stream) = self.stream_map.get_mut(&id) else {
            return Ok(());
        };

        stream.try_set_peer_reset();

        self.try_remove_stream(id);

        if self.reset_counter.tick() {
            return Err(Error::GoAway(Reason::ENHANCE_YOUR_CALM));
        }

        Ok(())
    }

    pub(super) fn request_body_drop(&mut self, id: StreamId, pending_window: usize) {
        let stream = self.stream_map.get_mut(&id).expect(STREAM_MUST_EXIST);

        match stream.maybe_close_recv(&mut self.frame_buf) {
            RecvClose::Cancel(size) => {
                let size = size + pending_window;
                self.queue.connection_window_update(size);
                self.queue.stream_window_update(id, size);
            }
            RecvClose::Close(size) => {
                self.queue.connection_window_update(size + pending_window);
            }
        }

        if let Some(remove) = stream.try_remove() {
            self.remove_stream(id, remove);
        }
    }

    fn stream_guard_drop(&mut self, id: StreamId) {
        let stream = self.stream_map.get_mut(&id).expect(STREAM_MUST_EXIST);

        stream.close_send();

        if let Some(remove) = stream.try_remove() {
            self.remove_stream(id, remove);
        }
    }

    fn try_remove_stream(&mut self, id: StreamId) {
        if let Some(stream) = self.stream_map.get_mut(&id) {
            stream.promote_cancel_to_close_recv();
            if let Some(remove) = stream.try_remove() {
                self.remove_stream(id, remove);
            }
        }
    }

    fn remove_stream(&mut self, id: StreamId, remove: Remove) {
        self.stream_map.remove(&id);
        if let Remove::Reset(err) = remove {
            self.queue.push(Message::Reset {
                stream_id: id,
                reason: err.reason(),
            });
            // Count client-caused resets (protocol errors, flow-control
            // violations, content-length mismatches) toward the sliding
            // window. INTERNAL_ERROR is server-originated and excluded.
            // When the limit is exceeded, queue GOAWAY and close the write
            // side so the connection drains and terminates.
            if !matches!(err, StreamError::InternalError) && self.reset_counter.tick() {
                self.go_away(Reason::ENHANCE_YOUR_CALM);
            }
        }
    }

    fn handle_data(&mut self, data: Data) -> Result<(), Error> {
        let id = data.stream_id();
        self.check_not_idle(id)?;

        let flow_len = data.flow_controlled_len();
        self.recv_window_dec(flow_len)?;

        let stream = self.stream_map.get_mut(&id).ok_or_else(|| {
            self.queue.connection_window_update(flow_len);
            Error::Reset(Reason::STREAM_CLOSED)
        })?;

        let end_stream = data.is_end_stream();
        let data = data.into_payload();

        let (conn_window, stream_window, want_remove) =
            match stream.try_recv_data(&mut self.frame_buf, data, flow_len, end_stream)? {
                RecvData::Queued(size) => {
                    // Padding isn't body-observable — auto-release the padding
                    // portion on both connection and stream windows now, so only
                    // the data portion is paced by application consumption.
                    (size, size, false)
                }
                RecvData::Discard(size) => {
                    let stream_window = if !end_stream { size } else { 0 };
                    (size, stream_window, end_stream)
                }
                // stream reseted. replenish connection window
                RecvData::StreamReset(size) => (size, 0, true),
            };

        self.queue.connection_window_update(conn_window);
        self.queue.stream_window_update(id, stream_window);

        // try remove stream from map in case RequestBody is dropped before reaching end_stream or rst_stream state
        if want_remove {
            self.try_remove_stream(id);
        }

        Ok(())
    }

    fn reset(&mut self, id: &StreamId) {
        self.stream_map
            .get_mut(id)
            .expect(STREAM_MUST_EXIST)
            .try_set_reset(StreamError::InternalError);
    }

    fn go_away(&mut self, reason: Reason) {
        if let Some(last_stream_id) = self.last_stream_id.try_go_away() {
            self.queue.push(Message::GoAway { last_stream_id, reason });
        }

        if reason != Reason::NO_ERROR {
            self.queue.close();
        }
    }

    pub(super) fn try_set_pending_ping(&mut self) -> io::Result<()> {
        if self.queue.keepalive_ping != KeepalivePing::Idle {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "h2 ping timeout"));
        }
        self.queue.keepalive_ping = KeepalivePing::Pending;
        Ok(())
    }
}

#[derive(Default)]
struct RemoteSettings {
    header_table_size: Option<Option<u32>>,
}

impl RemoteSettings {
    // Store incoming peer SETTINGS. Errors with ENHANCE_YOUR_CALM if a
    // previous SETTINGS has not yet been ACKed (CVE-2019-9515).
    fn try_update(&mut self, settings: Settings) -> Result<(), Error> {
        // Only one in flight peer settings is allowed. This is over restricted according
        // to RFC and multiple settings on wire should be allowed. That said in practice
        // only malicous peer would initliaze mutliple settings in short time burst.
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
    Data(Data),
    Trailer(headers::Headers<()>),
    Reset { stream_id: StreamId, reason: Reason },
    WindowUpdate { stream_id: StreamId, size: usize },
    GoAway { last_stream_id: StreamId, reason: Reason },
}

pub(super) struct WriterQueue {
    pub(super) messages: VecDeque<Message>,
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
    pub(super) fn connection_window_update(&mut self, size: usize) {
        self.pending_conn_window += size;
    }

    fn stream_window_update(&mut self, id: StreamId, size: usize) {
        if size > 0 {
            self.push(Message::WindowUpdate { stream_id: id, size })
        }
    }

    pub(super) fn push_data(&mut self, id: StreamId, payload: Bytes, end_stream: bool) {
        let mut data = Data::new(id, payload);
        data.set_end_stream(end_stream);
        self.push(Message::Data(data));
    }

    pub(super) fn push_trailers(&mut self, id: StreamId, trailers: HeaderMap) {
        let trailer = headers::Headers::trailers(id, trailers);
        self.push(Message::Trailer(trailer));
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

    fn try_recv(&mut self) -> Option<Message> {
        self.messages.pop_front()
    }

    fn is_closed(&self) -> bool {
        self.closed
    }
}

type Decoded = (Request<RequestExt<RequestBody>>, StreamId);

impl<'a, S> DecodeContext<'a, S> {
    /// Handle a stream-level error: queue RST_STREAM and replenish any
    /// connection window bytes consumed by the offending frame.
    /// Returns `Err(GoAway)` if the client has triggered too many resets.
    fn handle_stream_reset(&self, id: StreamId, reason: Reason) -> Result<(), Error> {
        let mut inner = self.ctx.borrow_mut();
        inner.queue.push(Message::Reset { stream_id: id, reason });
        if inner.reset_counter.tick() {
            return Err(Error::GoAway(Reason::ENHANCE_YOUR_CALM));
        }
        Ok(())
    }

    fn new(
        ctx: &'a Shared,
        service: &'a S,
        max_concurrent_streams: usize,
        recv_threshold: StreamRecvWindowThreshold,
        addr: SocketAddr,
        date: &'a DateTimeHandle,
    ) -> Self {
        Self {
            max_header_list_size: settings::DEFAULT_SETTINGS_MAX_HEADER_LIST_SIZE,
            max_concurrent_streams,
            recv_threshold,
            decoder: hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE),
            next_frame_len: 0,
            continuation: None,
            ctx,
            service,
            addr,
            date,
        }
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
                let data = Data::load(head, frame.freeze())?;
                self.ctx.borrow_mut().handle_data(data)?;
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
                            state.try_set_reset(StreamError::WindowUpdateZeroIncrement);
                        }
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
                            if state.send.window + incr > settings::MAX_INITIAL_WINDOW_SIZE as i64 {
                                state.try_set_reset(StreamError::WindowUpdateOverflow);
                            } else {
                                state.send.window += incr;
                                if window > 0 {
                                    state.send.wake();
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
                let go_away = GoAway::load(head.stream_id(), frame.as_ref())?;
                match go_away.reason() {
                    Reason::NO_ERROR => self.ctx.borrow_mut().go_away(Reason::NO_ERROR),
                    reason => return Err(Error::GoAway(reason)),
                }
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
            let delta = new_window - flow.send_stream_initial_window;
            flow.send_stream_initial_window = new_window;

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

        let flow = &mut *self.ctx.borrow_mut();
        if !flow.last_stream_id.check_idle(id) {
            let stream = flow
                .stream_map
                .get_mut(&id)
                .ok_or(Error::GoAway(Reason::STREAM_CLOSED))?;

            // pseudo is not checked for legitmacy and ignored when receiving trailers.

            match stream.try_recv_trailers(&mut flow.frame_buf, headers, end_stream)? {
                RecvData::Queued(_) => {}
                _ => flow.try_remove_stream(id),
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

        let body = RequestBody::new(id, content_length, Rc::clone(self.ctx), self.recv_threshold);

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

        while let Some(msg) = flow.queue.try_recv() {
            match msg {
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
                Message::WindowUpdate { stream_id, size } => WindowUpdate::new(stream_id, size as _).encode(write_buf),
                Message::GoAway { last_stream_id, reason } => {
                    GoAway::new(last_stream_id, reason).encode(write_buf);
                    // GoAway may be graceful (queue stays open to drain in-flight frames)
                    // or forceful (queue closed). The pusher decides via FlowControl::go_away;
                    // we keep draining either way.
                }
            }
        }

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
        } else if flow.queue.is_closed() {
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

    let head_method = req.method() == Method::HEAD;

    let res = match service.call(req).await {
        Ok(res) => res,
        Err(_) => {
            ctx.borrow_mut().reset(&stream_id);
            return;
        }
    };

    let (mut parts, body) = res.into_parts();

    super::strip_connection_headers::<false>(&mut parts.headers);

    if !parts.headers.contains_key(DATE) {
        let date = date.with_date_header(Clone::clone);
        parts.headers.insert(DATE, date);
    }

    let end_stream = match (head_method, body.size_hint()) {
        (true, _) => true,
        (false, SizeHint::None) => true,
        (false, size) => {
            if let SizeHint::Exact(size) = size {
                parts.headers.entry(CONTENT_LENGTH).or_insert_with(|| size.into());
            }
            false
        }
    };

    let pseudo = headers::Pseudo::response(parts.status);
    let mut headers = headers::Headers::new(stream_id, pseudo, parts.headers);

    if end_stream {
        headers.set_end_stream();
    }

    ctx.borrow_mut().queue.push(Message::Head(headers));

    if !end_stream {
        let mut body = pin!(body);

        loop {
            match poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
                None => break ctx.borrow_mut().queue.push_end_stream(guard.stream_id),
                Some(Err(e)) => {
                    error!("body error: {:?}", e);
                    break guard.reset();
                }
                Some(Ok(Frame::Data(bytes))) => {
                    if guard.send_data(bytes, body.is_end_stream()).await {
                        break;
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
        self.ctx.borrow_mut().stream_guard_drop(self.stream_id);
    }
}

struct SendData<'a> {
    data: Bytes,
    end_stream: bool,
    stream_id: StreamId,
    flow: &'a Shared,
}

impl Future for SendData<'_> {
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        let flow = &mut *this.flow.borrow_mut();

        loop {
            let len = this.data.len();

            let opt = ready!(
                flow.stream_map
                    .get_mut(&this.stream_id)
                    .expect(STREAM_MUST_EXIST)
                    .poll_send_window(len, flow.send_connection_window, cx)
            );

            let Some(Ok(aval)) = opt else {
                return Poll::Ready(true);
            };

            flow.send_connection_window -= aval;

            let all_consumed = aval == len;

            let payload = if all_consumed {
                mem::take(&mut this.data)
            } else {
                this.data.split_to(aval)
            };

            let end_stream = all_consumed && this.end_stream;

            flow.queue.push_data(this.stream_id, payload, end_stream);

            if end_stream {
                return Poll::Ready(true);
            } else if all_consumed {
                return Poll::Ready(false);
            }
        }
    }
}

impl StreamGuard<'_> {
    // return true to inform outer loop to break
    async fn send_data(&self, data: Bytes, end_stream: bool) -> bool {
        if data.is_empty() && !end_stream {
            tracing::warn!("response body should not yield empty Frame::Data unless it's the last chunk of Body");
            return false;
        }

        SendData {
            data,
            end_stream,
            stream_id: self.stream_id,
            flow: self.ctx,
        }
        .await
    }

    fn send_trailers(&self, trailers: HeaderMap) {
        self.ctx.borrow_mut().queue.push_trailers(self.stream_id, trailers);
    }

    fn reset(&self) {
        self.ctx.borrow_mut().reset(&self.stream_id);
    }
}

/// Maximum number of client-caused resets allowed within `RESET_WINDOW`
/// before the connection is closed with GOAWAY(ENHANCE_YOUR_CALM).
/// Matches the h2 crate's DEFAULT_REMOTE_RESET_STREAM_MAX.
const RESET_MAX: usize = 20;
/// Sliding window duration for the reset counter. Resets older than this
/// are expired and no longer counted.
const RESET_WINDOW: Duration = Duration::from_secs(30);

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

    let max_concurrent_streams = config.h2_max_concurrent_streams as usize;

    let mut flow = FlowControl {
        // Send windows start at RFC 7540 §6.9.2 default (65535) until the
        // peer's SETTINGS_INITIAL_WINDOW_SIZE is received and applied.
        send_connection_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as usize,
        send_stream_initial_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as i64,
        max_frame_size: settings::DEFAULT_MAX_FRAME_SIZE as usize,
        recv_stream_initial_window: config.h2_initial_window_size as usize,
        recv_connection_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as usize,
        stream_map: HashMap::with_capacity_and_hasher(max_concurrent_streams, NoHashBuilder::default()),
        reset_counter: ResetCounter::new(RESET_MAX, RESET_WINDOW),
        last_stream_id: LastStreamId::new(),
        queue: WriterQueue::new(),
        frame_buf: FrameBuffer::new(),
    };

    let (mut read_buf, mut write_buf) = flow
        .handshake::<READ_BUF_LIMIT>(&io, read_buf, &settings, ka.as_mut())
        .await?;

    let shared = Rc::new(RefCell::new(flow));

    let recv_threshold = StreamRecvWindowThreshold::from(&settings);
    let mut ctx = DecodeContext::new(&shared, service, max_concurrent_streams, recv_threshold, addr, date);
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
                        Ok(n) if n > 0 => loop {
                            let reason = match ctx.try_decode(&mut read_buf) {
                                Ok(Some((req, id))) => {
                                    queue.push(response_task(req, id, ctx.service, ctx.ctx, ctx.date));
                                    continue;
                                }
                                Ok(None) => break,
                                Err(Error::Reset(_)) => unreachable!(
                                    "reset error must be handled at scope where it's associated StreamId exsits"
                                ),
                                Err(Error::GoAway(reason)) => reason,
                                Err(Error::Hpack(_)) => Reason::COMPRESSION_ERROR,
                                Err(Error::MalformedMessage) => Reason::PROTOCOL_ERROR,
                            };
                            queue.clear();
                            ctx.ctx.borrow_mut().go_away(reason);
                            break;
                        },
                        res => break ShutDown::ReadClosed(res.map(|_| ())),
                    };

                    read_task.set(read_io(read_buf, &io));
                }
                SelectOutput::A(SelectOutput::A(SelectOutput::B(_))) => unreachable!(),
                SelectOutput::A(SelectOutput::B(res)) => break ShutDown::WriteClosed(res),
                SelectOutput::B(Ok(_)) => {}
                SelectOutput::B(Err(e)) => break ShutDown::Timeout(e),
            }
        };

        Box::pin(async {
            let read_res = match shutdown {
                ShutDown::WriteClosed(res) => return res,
                ShutDown::Timeout(err) => return Err(err),
                ShutDown::ReadClosed(res) => {
                    {
                        let mut flow = shared.borrow_mut();
                        for state in flow.stream_map.values_mut() {
                            state.try_set_peer_reset();
                        }
                    }

                    res
                }
            };

            loop {
                if queue.is_empty() {
                    shared.borrow_mut().queue.close();
                }

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

impl FlowControl {
    /// Perform the HTTP/2 connection handshake: validate the client preface,
    /// then send our SETTINGS frame. Returns the read buffer (with preface
    /// consumed) and the write buffer (ready for reuse).
    #[cold]
    #[inline(never)]
    fn handshake<'a, const LIMIT: usize>(
        &'a mut self,
        io: &'a (impl AsyncBufRead + AsyncBufWrite),
        buf: BytesMut,
        settings: &'a settings::Settings,
        timer: Pin<&'a mut KeepAlive>,
    ) -> BoxedFuture<'a, io::Result<(BytesMut, BytesMut)>> {
        Box::pin(async move {
            async {
                // No cap during preface: the buffer is tiny and always fully drained.
                let (mut read_buf, res) = prefix_check::<LIMIT>(buf, io).await;
                res?;
                read_buf.advance(PREFACE.len());

                let mut write_buf = BytesMut::new();
                settings.encode(&mut write_buf);

                let delta =
                    (self.recv_stream_initial_window as u32).saturating_sub(settings::DEFAULT_INITIAL_WINDOW_SIZE);

                if delta > 0 {
                    WindowUpdate::new(StreamId::ZERO, delta).encode(&mut write_buf);
                    self.recv_connection_window += delta as usize;
                };

                let (res, write_buf) = write_io(write_buf, io).await;
                res?;

                Ok((read_buf, write_buf))
            }
            .timeout(timer)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "h2 handshake timeout"))?
        })
    }
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
