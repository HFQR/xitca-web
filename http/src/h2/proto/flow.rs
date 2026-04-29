use core::{
    cell::RefCell,
    cmp, mem,
    task::{Context, Poll, Waker, ready},
    time::Duration,
};

use std::{
    collections::{HashMap, VecDeque},
    io,
    rc::Rc,
};

use xitca_unsafe_collection::no_hash::NoHashBuilder;

use crate::{
    body::SizeHint,
    bytes::{BufMut, Bytes, BytesMut},
    http::{Method, Protocol, Request, Version, header::HeaderMap, uri},
};

use super::{
    error::Error as ProtoError,
    frame::{
        data::Data,
        go_away::GoAway,
        head,
        headers::{Headers, ResponsePseudo},
        ping::Ping,
        reason::Reason,
        reset::Reset,
        settings::{self, Settings},
        stream_id::StreamId,
        window_update::WindowUpdate,
    },
    hpack,
    last_stream_id::LastStreamId,
    reset_counter::ResetCounter,
    size::BodySize,
    stream::{RecvData, Stream, StreamError, TryRemove},
    threshold::StreamRecvWindowThreshold,
};

const STREAM_MUST_EXIST: &str = "Stream MUST NOT be removed while RequestBody or response_task is still alive";

pub(crate) type Frame = crate::body::Frame<Bytes>;
pub(crate) type FrameBuffer = crate::h2::util::FrameBuffer<Frame>;

pub(crate) type DecodedRequest = (Request<(SizeHint, Option<Protocol>)>, StreamId);

pub(crate) type FlowControlClone = Rc<FlowControlLock>;
pub(crate) type FlowControlLock = RefCell<FlowControl>;

pub(crate) struct FlowControl {
    max_concurrent_streams: usize,
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
    recv_threshold: StreamRecvWindowThreshold,
    /// Per-stream state. Inserted when HEADERS arrives or response body starts;
    /// removed when both sides are done or on RST_STREAM.
    stream_map: HashMap<StreamId, Stream, NoHashBuilder>,
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
    queue: WriterQueue,
    /// Shared slab backing all per-stream recv frame deques, avoiding
    /// per-stream `VecDeque` allocations.
    frame_buf: FrameBuffer,
}

impl FlowControl {
    /// Maximum number of client-caused resets allowed within `RESET_WINDOW`
    /// before the connection is closed with GOAWAY(ENHANCE_YOUR_CALM).
    /// Matches the h2 crate's DEFAULT_REMOTE_RESET_STREAM_MAX.
    const RESET_MAX: usize = 20;
    /// Sliding window duration for the reset counter. Resets older than this
    /// are expired and no longer counted.
    const RESET_WINDOW: Duration = Duration::from_secs(30);

    pub(crate) fn new(settings: &Settings) -> Self {
        let recv_stream_initial_window = settings.initial_window_size().unwrap() as _;
        let max_concurrent_streams = settings.max_concurrent_streams().unwrap() as _;
        let recv_threshold = StreamRecvWindowThreshold::from(settings);

        Self {
            max_concurrent_streams,
            // Send windows start at RFC 7540 §6.9.2 default (65535) until the
            // peer's SETTINGS_INITIAL_WINDOW_SIZE is received and applied.
            send_connection_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as usize,
            send_stream_initial_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as i64,
            max_frame_size: settings::DEFAULT_MAX_FRAME_SIZE as usize,
            recv_stream_initial_window,
            recv_connection_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as usize,
            recv_threshold,
            stream_map: HashMap::with_capacity_and_hasher(max_concurrent_streams, NoHashBuilder::default()),
            reset_counter: ResetCounter::new(Self::RESET_MAX, Self::RESET_WINDOW),
            last_stream_id: LastStreamId::new(),
            queue: WriterQueue::new(),
            frame_buf: FrameBuffer::new(),
        }
    }

    fn insert_stream(&mut self, id: StreamId, end_stream: bool, content_length: SizeHint) {
        let stream = Stream::new(
            self.send_stream_initial_window,
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
        let conn_window_positive = self.send_connection_window > 0;
        for stream in self.stream_map.values_mut() {
            stream.send_window_update(delta, conn_window_positive);
        }
    }

    pub(crate) fn request_body_drop(&mut self, id: StreamId, pending_window: usize) {
        let stream = self.stream_map.get_mut(&id).expect(STREAM_MUST_EXIST);

        let window = stream.maybe_close_recv(&mut self.frame_buf) + pending_window;

        let mut wake = window > 0;

        self.queue.connection_window_update(window);

        let remove = stream.try_remove();

        if let Err(err) = self.remove_stream(id, remove) {
            wake = true;

            if let Err(err) = self.try_push_reset(id, err.reason()) {
                self.go_away(err);
            }
        }

        // body is detached from Decode/EncodeContext. wake up WriterQueue
        // if there are connection window and/or reset/goaway message scheduled
        // to be sent
        if wake {
            self.queue.wake();
        }
    }

    pub(crate) fn response_task_done(&mut self, id: StreamId) -> Result<(), ()> {
        let stream = self.stream_map.get_mut(&id).expect(STREAM_MUST_EXIST);

        stream.close_send();

        let remove = stream.try_remove();

        if let Err(err) = self.remove_stream(id, remove) {
            if let Err(err) = self.try_push_reset(id, err.reason()) {
                self.go_away(err);
                return Err(());
            }
        }

        Ok(())
    }

    fn inline_remove_stream(&mut self, id: StreamId) -> Result<(), Error> {
        if let Some(stream) = self.stream_map.get_mut(&id) {
            stream.promote_cancel_to_close_recv();
            let remove = stream.try_remove();
            self.remove_stream(id, remove)?;
        };

        Ok(())
    }

    fn remove_stream(&mut self, id: StreamId, remove: TryRemove) -> Result<(), StreamError> {
        let res = match remove {
            TryRemove::Keep => return Ok(()),
            TryRemove::ResetKeep(err) => return Err(err),
            TryRemove::ResetRemove(err) => Err(err),
            TryRemove::Remove => Ok(()),
        };

        self.stream_map.remove(&id);

        res
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn try_push_reset(&mut self, id: StreamId, reason: Reason) -> Result<(), Error> {
        // Count client-caused resets (protocol errors, flow-control
        // violations, content-length mismatches) toward the sliding
        // window. Server-originated variants (INTERNAL_ERROR from
        // response body errors, Cancel from RequestBody drop) are
        // excluded. When the limit is exceeded, queue GOAWAY and close
        // the write side so the connection drains and terminates.

        if !matches!(reason, Reason::INTERNAL_ERROR | Reason::NO_ERROR) {
            self.try_tick_reset()?;
        }

        self.queue.push(Message::Reset { stream_id: id, reason });
        Ok(())
    }

    pub(crate) fn recv_header(&mut self, id: StreamId, headers: Headers) -> Result<Option<DecodedRequest>, Error> {
        let end_stream = headers.is_end_stream();

        let (pseudo, headers) = headers.into_parts();

        if !self.last_stream_id.check_idle(id) {
            let stream = self
                .stream_map
                .get_mut(&id)
                .ok_or(Error::GoAway(Reason::STREAM_CLOSED))?;

            // pseudo is not checked for legitmacy and ignored when receiving trailers.

            match stream.try_recv_trailers(&mut self.frame_buf, headers, end_stream)? {
                RecvData::Queued(_) => {}
                _ => self.inline_remove_stream(id)?,
            }
            return Ok(None);
        }

        // Validate and advance the stream-ID boundary before any
        // application-level checks. This ensures protocol violations
        // (non-client-initiated, non-monotonic) produce GOAWAY rather
        // than being masked by REFUSED_STREAM.
        // Saturated (post-GOAWAY): silently drop the frame.
        if self.try_set_last_stream_id(id)?.is_none() {
            return Ok(None);
        }

        if self.stream_map.len() >= self.max_concurrent_streams {
            return Err(Error::Reset(Reason::REFUSED_STREAM));
        }

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

        let mut req = Request::new((content_length, protocol));
        *req.version_mut() = Version::HTTP_2;
        *req.headers_mut() = headers;
        *req.method_mut() = method;

        if let Ok(uri) = uri::Uri::from_parts(uri_parts) {
            *req.uri_mut() = uri;
        }

        self.insert_stream(id, end_stream, content_length);

        Ok(Some((req, id)))
    }

    pub(crate) fn try_set_last_stream_id(&mut self, id: StreamId) -> Result<Option<()>, Error> {
        self.last_stream_id.try_set(id).map_err(Into::into)
    }

    pub(crate) fn recv_data(&mut self, data: Data) -> Result<(), Error> {
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
            self.inline_remove_stream(id)?;
        }

        Ok(())
    }

    pub(crate) fn recv_window_update(&mut self, window: WindowUpdate) -> Result<(), Error> {
        let id = window.stream_id();
        self.check_not_idle(id)?;

        match (window.size_increment(), id) {
            (0, StreamId::ZERO) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
            (0, id) => {
                if let Some(state) = self.stream_map.get_mut(&id) {
                    state.try_set_reset(StreamError::WindowUpdateZeroIncrement);
                }
            }
            (incr, StreamId::ZERO) => {
                let incr = incr as usize;

                if self.send_connection_window + incr > settings::MAX_INITIAL_WINDOW_SIZE {
                    return Err(Error::GoAway(Reason::FLOW_CONTROL_ERROR));
                }

                let was_zero = self.send_connection_window == 0;
                self.send_connection_window += incr;

                // Only wake streams if the connection window just became
                // available — if it was already >0, no stream was blocked on it.
                if was_zero {
                    self.update_and_wake_send_streams(0);
                }
            }
            (incr, id) => {
                let incr = incr as i64;
                let conn_window_positive = self.send_connection_window > 0;
                if let Some(stream) = self.stream_map.get_mut(&id) {
                    stream.try_send_window_update(incr, conn_window_positive);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn recv_ping(&mut self, ping: Ping) {
        if ping.is_ack {
            // ACK for our keepalive PING: return to Idle so `PingPong::tick`
            // does not time out on the next tick.
            self.queue.keepalive_ping = KeepalivePing::Idle;
        } else {
            // Client-initiated PING: always overwrite; we reply with ACK.
            self.queue.pending_client_ping = Some(ping.payload);
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn recv_reset(&mut self, reset: Reset) -> Result<(), Error> {
        let id = reset.stream_id();

        if id.is_zero() {
            return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
        }

        self.check_not_idle(id)?;

        // A RST_STREAM is "premature" if the stream is still tracked — either
        // the response task is running or the request body is still being
        // received. Count these to detect rapid-reset abuse (CVE-2023-44487).
        self.try_tick_reset()?;

        let Some(stream) = self.stream_map.get_mut(&id) else {
            return Ok(());
        };

        stream.try_set_peer_reset();

        self.inline_remove_stream(id)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn internal_reset(&mut self, id: &StreamId) {
        self.stream_map
            .get_mut(id)
            .expect(STREAM_MUST_EXIST)
            .try_set_reset(StreamError::InternalError);
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn go_away(&mut self, err: Error) -> bool {
        let Error::GoAway(reason) = err else {
            unreachable!("Error::Reset MUST not be handled as GO_AWAY frame")
        };

        if let Some(last_stream_id) = self.last_stream_id.try_go_away() {
            self.queue.push(Message::GoAway { last_stream_id, reason });
        }

        let fatal = reason != Reason::NO_ERROR;

        if fatal {
            self.queue.close();
        }

        fatal
    }

    pub(crate) fn poll_stream_frame(
        &mut self,
        id: &StreamId,
        pending_window: &mut usize,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame, StreamError>>> {
        let stream = self.stream_map.get_mut(id).expect(STREAM_MUST_EXIST);

        stream.poll_frame(&mut self.frame_buf, cx).map_ok(|frame| {
            if let Some(bytes) = frame.data_ref() {
                *pending_window += bytes.len();

                if *pending_window >= self.recv_threshold {
                    let window = mem::replace(pending_window, 0);
                    stream.recv_window_update(window);
                    self.queue.connection_window_update(window);
                    self.queue.stream_window_update(*id, window);
                }
            }
            frame
        })
    }

    pub(crate) fn is_recv_end_stream(&self, id: &StreamId) -> bool {
        self.stream_map.get(id).expect(STREAM_MUST_EXIST).is_recv_end_stream()
    }

    pub(super) fn try_set_pending_ping(&mut self) -> io::Result<()> {
        self.queue.keepalive_ping.try_set_pending_ping()
    }

    fn try_tick_reset(&mut self) -> Result<(), Error> {
        if self.reset_counter.tick() {
            Err(Error::GoAway(Reason::ENHANCE_YOUR_CALM))
        } else {
            Ok(())
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn recv_setting(&mut self, head: head::Head, frame: &BytesMut) -> Result<(), Error> {
        let setting = Settings::load(head, frame)?;

        if setting.is_ack() {
            return Ok(());
        }

        if let Some(new_window) = setting.initial_window_size() {
            let new_window = new_window as i64;
            let delta = new_window - self.send_stream_initial_window;
            self.send_stream_initial_window = new_window;

            if delta > 0 {
                for stream in self.stream_map.values() {
                    stream
                        .send_window_check(delta)
                        .map_err(|err| Error::GoAway(err.reason()))?;
                }
            }

            if delta != 0 {
                self.update_and_wake_send_streams(delta);
            }
        }

        if let Some(frame_size) = setting.max_frame_size() {
            self.max_frame_size = frame_size as usize;
        }

        // Record the pending ACK. A second SETTINGS before the first is ACKed
        // is a protocol violation and returns ENHANCE_YOUR_CALM (CVE-2019-9515).
        self.queue.pending_settings.try_update(setting)
    }

    pub(crate) fn poll_encode(
        &mut self,
        write_buf: &mut BytesMut,
        encoder: &mut hpack::Encoder,
        cx: &mut Context<'_>,
    ) -> Poll<bool> {
        // remote_setting can contain updated states for following encoding
        self.queue.pending_settings.encode(encoder, write_buf);

        while let Some(msg) = self.queue.try_recv() {
            match msg {
                Message::Head(headers) => {
                    let frame_size = self.max_frame_size;
                    let mut cont = headers.encode(encoder, &mut write_buf.limit(frame_size));
                    while let Some(c) = cont {
                        cont = c.encode(&mut write_buf.limit(frame_size));
                    }
                }
                Message::Trailer(headers) => {
                    let frame_size = self.max_frame_size;
                    let mut cont = headers.encode(encoder, &mut write_buf.limit(frame_size));
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

        let pending = mem::replace(&mut self.queue.pending_conn_window, 0);
        if pending > 0 {
            self.recv_connection_window += pending;
            WindowUpdate::new(StreamId::zero(), pending as _).encode(write_buf);
        }

        // Encode a client PING ACK if one is waiting (take-and-clear).
        if let Some(payload) = self.queue.pending_client_ping.take() {
            Ping::new(payload, true).encode(write_buf);
        }

        self.queue.keepalive_ping.encode(write_buf);

        if !write_buf.is_empty() {
            Poll::Ready(true)
        } else if self.queue.is_closed() {
            Poll::Ready(false)
        } else {
            self.queue.register(cx);
            Poll::Pending
        }
    }

    pub(crate) fn send_headers(&mut self, headers: Headers<ResponsePseudo>) {
        self.queue.push(Message::Head(headers));
    }

    pub(crate) fn poll_send_data(
        &mut self,
        id: StreamId,
        data: &mut Bytes,
        end_stream: bool,
        cx: &mut Context<'_>,
    ) -> Poll<Option<()>> {
        let stream = self.stream_map.get_mut(&id).expect(STREAM_MUST_EXIST);

        loop {
            let len = data.len();

            let cap = cmp::min(self.send_connection_window, self.max_frame_size);

            let opt = ready!(stream.poll_send_window(len, cap, cx));

            let Some(Ok(aval)) = opt else {
                return Poll::Ready(Some(()));
            };

            self.send_connection_window -= aval;

            let all_consumed = aval == len;

            let payload = if all_consumed {
                mem::take(data)
            } else {
                data.split_to(aval)
            };

            let end_stream = all_consumed && end_stream;

            self.queue.push_data(id, payload, end_stream);

            if end_stream {
                return Poll::Ready(Some(()));
            } else if all_consumed {
                return Poll::Ready(None);
            }
        }
    }

    pub(crate) fn send_trailers(&mut self, id: StreamId, trailers: HeaderMap) {
        self.queue.push_trailers(id, trailers);
    }

    pub(crate) fn send_end_stream(&mut self, id: StreamId) {
        self.queue.push_end_stream(id);
    }

    pub(crate) fn close_write_queue(&mut self) {
        self.queue.close();
    }

    pub(crate) fn reset_all_stream(&mut self, res: &io::Result<()>) {
        let stream_err = if res.is_ok() {
            StreamError::GoAway
        } else {
            StreamError::Io
        };

        for stream in self.stream_map.values_mut() {
            stream.try_set_reset(stream_err);
        }
    }

    pub(crate) fn encode_init_window_update(&mut self, buf: &mut BytesMut) {
        let delta = (self.recv_stream_initial_window as u32).saturating_sub(settings::DEFAULT_INITIAL_WINDOW_SIZE);

        if delta > 0 {
            WindowUpdate::new(StreamId::ZERO, delta).encode(buf);
            self.recv_connection_window += delta as usize;
        };
    }
}

struct WriterQueue {
    messages: VecDeque<Message>,
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
    waker: Option<Waker>,
}

impl WriterQueue {
    fn new() -> Self {
        Self {
            messages: VecDeque::new(),
            closed: false,
            pending_settings: RemoteSettings::default(),
            pending_conn_window: 0,
            keepalive_ping: KeepalivePing::Idle,
            pending_client_ping: None,
            waker: None,
        }
    }

    fn push(&mut self, msg: Message) {
        self.messages.push_back(msg);
    }

    /// Accumulate a connection-level WINDOW_UPDATE into `pending_conn_window`.
    /// Flushed as a single connection frame after `poll_encode` drains the queue.
    fn connection_window_update(&mut self, size: usize) {
        self.pending_conn_window += size;
    }

    fn stream_window_update(&mut self, id: StreamId, size: usize) {
        if size > 0 {
            self.push(Message::WindowUpdate { stream_id: id, size })
        }
    }

    fn push_data(&mut self, id: StreamId, payload: Bytes, end_stream: bool) {
        let mut data = Data::new(id, payload);
        data.set_end_stream(end_stream);
        self.push(Message::Data(data));
    }

    fn push_trailers(&mut self, id: StreamId, trailers: HeaderMap) {
        let trailer = Headers::trailers(id, trailers);
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

    fn register(&mut self, cx: &mut Context<'_>) {
        if self
            .waker
            .as_ref()
            .filter(|waker| waker.will_wake(cx.waker()))
            .is_none()
        {
            self.waker = Some(cx.waker().clone());
        }
    }

    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

#[derive(Default)]
struct RemoteSettings {
    header_table_size: Option<Option<u32>>,
}

impl RemoteSettings {
    // Store incoming peer SETTINGS. Errors with ENHANCE_YOUR_CALM if a
    // previous SETTINGS has not yet been ACKed (CVE-2019-9515).
    #[cold]
    #[inline(never)]
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
enum KeepalivePing {
    Idle,
    Pending,
    InFlight,
}

impl KeepalivePing {
    fn encode(&mut self, write_buf: &mut BytesMut) {
        // Encode our keepalive PING if it is queued but not yet sent, then
        // transition to InFlight so we do not re-send it on the next pass.
        if matches!(self, KeepalivePing::Pending) {
            Ping::new([0u8; 8], false).encode(write_buf);
            *self = KeepalivePing::InFlight;
        }
    }

    fn try_set_pending_ping(&mut self) -> io::Result<()> {
        if !matches!(self, KeepalivePing::Idle) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "h2 ping timeout"));
        }
        *self = KeepalivePing::Pending;
        Ok(())
    }
}

pub(crate) enum Error {
    Reset(Reason),
    GoAway(Reason),
}

impl From<ProtoError> for Error {
    fn from(e: ProtoError) -> Self {
        if e.is_go_away() {
            Self::GoAway(e.reason())
        } else {
            Self::Reset(e.reason())
        }
    }
}

impl From<StreamError> for Error {
    fn from(err: StreamError) -> Self {
        Self::Reset(err.reason())
    }
}

enum Message {
    Head(Headers<ResponsePseudo>),
    Data(Data),
    Trailer(Headers<()>),
    Reset { stream_id: StreamId, reason: Reason },
    WindowUpdate { stream_id: StreamId, size: usize },
    GoAway { last_stream_id: StreamId, reason: Reason },
}
