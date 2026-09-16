use core::{
    cell::RefCell,
    mem,
    task::{Context, Poll, ready},
    time::Duration,
};

use std::{collections::HashMap, io, rc::Rc};

use xitca_unsafe_collection::no_hash::NoHashBuilder;

use crate::{
    body::SizeHint,
    bytes::{Bytes, BytesMut},
    http::{Method, Protocol, Request, Version, header::HeaderMap, uri},
};

use super::{
    codec::EncodeContext,
    error::Error as ProtoError,
    frame::{
        data::Data,
        headers::{Headers, ResponsePseudo},
        ping::Ping,
        reason::Reason,
        reset::Reset,
        settings::Settings,
        stream_id::StreamId,
        window_update::WindowUpdate,
    },
    last_stream_id::LastStreamId,
    reset_counter::ResetCounter,
    size::BodySize,
    stream::{RecvData, Stream, StreamError, TryRemove},
    threshold::RecvWindowThreshold,
    window::{RecvWindow, SendWindow},
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
    send_connection_window: SendWindow,
    /// Default send-window for new streams (RFC 9113 §6.9.2).
    send_stream_initial_window: SendWindow,
    /// Default recv-window for new streams
    recv_stream_initial_window: RecvWindow,
    /// Remaining bytes we are willing to receive on the whole connection.
    recv_connection_window: RecvWindow,
    recv_stream_threshold: RecvWindowThreshold,
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
    /// causes new-stream HEADERS to be silently dropped (RFC 9113 §6.8)
    /// and signals the dispatcher's main loop that the graceful drain
    /// can terminate when the in-flight queue empties.
    last_stream_id: LastStreamId,
    encode: EncodeContext,
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
        let recv_stream_initial_window = RecvWindow::new(settings.initial_window_size().unwrap());
        let max_concurrent_streams = settings.max_concurrent_streams().unwrap() as _;

        Self {
            max_concurrent_streams,
            // Send windows start at RFC 9113 §6.9.2 default (65535) until the
            // peer's SETTINGS_INITIAL_WINDOW_SIZE is received and applied.
            send_connection_window: SendWindow::default(),
            send_stream_initial_window: SendWindow::default(),
            recv_stream_initial_window,
            recv_connection_window: RecvWindow::default(),
            recv_stream_threshold: RecvWindowThreshold::from(recv_stream_initial_window),
            stream_map: HashMap::with_capacity_and_hasher(max_concurrent_streams, NoHashBuilder::default()),
            reset_counter: ResetCounter::new(Self::RESET_MAX, Self::RESET_WINDOW),
            last_stream_id: LastStreamId::new(),
            encode: EncodeContext::new(),
            frame_buf: FrameBuffer::new(),
        }
    }

    fn check_not_idle(&self, id: StreamId) -> Result<(), Error> {
        if self.last_stream_id.check_idle(id) {
            Err(Error::GoAway(Reason::PROTOCOL_ERROR))
        } else {
            Ok(())
        }
    }

    fn recv_window_dec(&mut self, len: RecvWindow) -> Result<(), Error> {
        self.recv_connection_window
            .checked_sub(len)
            .map_err(|_| Error::GoAway(Reason::FLOW_CONTROL_ERROR))
    }

    /// Apply `delta` to every active send stream's window and wake any that
    /// now have a positive window. Use `delta = 0` to wake without changing
    /// windows (e.g. after a connection-level WINDOW_UPDATE).
    fn update_and_wake_send_streams(&mut self, delta: SendWindow) {
        let conn_window_positive = self.send_connection_window.is_positive();
        for stream in self.stream_map.values_mut() {
            stream.send_window_update(delta, conn_window_positive);
        }
    }

    pub(crate) fn request_body_drop(&mut self, id: StreamId) {
        let stream = self.stream_map.get_mut(&id).expect(STREAM_MUST_EXIST);

        // Consumed bytes are already counted at connection scope. Release only
        // the still-unconsumed queue here to avoid crediting those bytes twice.
        let window = stream.maybe_close_recv(&mut self.frame_buf);

        let mut wake = window != RecvWindow::ZERO;

        self.encode.connection_window_update(window);

        let remove = stream.try_remove();

        if let Err(err) = self.remove_stream(id, remove) {
            wake = true;

            if let Err(err) = self.try_push_reset(id, err.reason()) {
                self.go_away(err);
            }
        }

        // body is detached from Decode/EncodeContext. wake up the encoder
        // if there are connection window and/or reset/goaway message scheduled
        // to be sent
        if wake {
            self.encode.wake();
        }
    }

    pub(crate) fn response_task_done(&mut self, id: StreamId) -> Result<(), ()> {
        let stream = self.stream_map.get_mut(&id).expect(STREAM_MUST_EXIST);

        stream.close_send();

        let remove = stream.try_remove();

        if let Err(err) = self.remove_stream(id, remove)
            && let Err(err) = self.try_push_reset(id, err.reason())
        {
            self.go_away(err);
            return Err(());
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

        self.encode.push_reset(id, reason);
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
        // RFC 9113 §8.5: regular CONNECT MUST NOT include :scheme or :path.
        // RFC 9113 §8.3.1: non-CONNECT MUST include :scheme and non-empty :path.
        let mut uri_parts = uri::Parts::default();

        if let Some(authority) = pseudo.authority
            && let Ok(a) = uri::Authority::from_maybe_shared(authority.into_inner())
        {
            uri_parts.authority = Some(a);
        }

        match (is_strict_connect, pseudo.scheme) {
            // RFC 9113 §8.3.1: non-CONNECT MUST include :scheme.
            // RFC 9113 §8.5: regular CONNECT MUST NOT include :scheme.
            (true, Some(_)) | (false, None) => return Err(Error::Reset(Reason::PROTOCOL_ERROR)),
            (false, Some(scheme)) if uri_parts.authority.is_some() => {
                if let Ok(s) = uri::Scheme::try_from(scheme.as_str()) {
                    uri_parts.scheme = Some(s);
                }
            }
            _ => {}
        }

        match (is_strict_connect, pseudo.path) {
            // RFC 9113 §8.3.1: non-CONNECT MUST include :path.
            // RFC 9113 §8.5: regular CONNECT MUST NOT include :path.
            (true, Some(_)) | (false, None) => return Err(Error::Reset(Reason::PROTOCOL_ERROR)),
            (_, Some(path)) if !path.is_empty() => {
                if let Ok(pq) = uri::PathAndQuery::from_maybe_shared(path.into_inner()) {
                    uri_parts.path_and_query = Some(pq);
                }
            }
            // RFC 9113 §8.3.1: non-CONNECT MUST include non empty :path.
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

        let stream = Stream::new(
            self.send_stream_initial_window,
            self.recv_stream_initial_window,
            content_length,
            end_stream,
        );

        self.stream_map.insert(id, stream);

        Ok(Some((req, id)))
    }

    pub(crate) fn try_set_last_stream_id(&mut self, id: StreamId) -> Result<Option<()>, Error> {
        self.last_stream_id.try_set(id).map_err(Into::into)
    }

    pub(crate) fn recv_data(&mut self, data: Data) -> Result<(), Error> {
        let id = data.stream_id();
        self.check_not_idle(id)?;

        // RFC 9113 §6.5.2: SETTINGS_MAX_FRAME_SIZE is 24-bit, so flow-controlled
        // length always fits in u32.
        let flow_len = data.flow_controlled_len() as u32;
        let flow_len = RecvWindow::new(flow_len);
        self.recv_window_dec(flow_len)?;

        let stream = self.stream_map.get_mut(&id).ok_or_else(|| {
            self.encode.connection_window_update(flow_len);
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
                    let stream_window = if !end_stream { size } else { RecvWindow::ZERO };
                    (size, stream_window, end_stream)
                }
                // stream reseted. replenish connection window
                RecvData::StreamReset(size) => (size, RecvWindow::ZERO, true),
            };

        self.encode.connection_window_update(conn_window);
        self.encode.stream_window_update(id, stream_window);

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
                let was_zero = self.send_connection_window == SendWindow::ZERO;

                self.send_connection_window
                    .try_inc(SendWindow::from_u32(incr))
                    .map_err(|_| Error::GoAway(Reason::FLOW_CONTROL_ERROR))?;

                // Only wake streams if the connection window just became
                // available — if it was already >0, no stream was blocked on it.
                if was_zero {
                    self.update_and_wake_send_streams(SendWindow::ZERO);
                }
            }
            (incr, id) => {
                let conn_window_positive = self.send_connection_window.is_positive();
                if let Some(stream) = self.stream_map.get_mut(&id) {
                    stream.try_send_window_update(SendWindow::from_u32(incr), conn_window_positive);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn recv_ping(&mut self, ping: Ping) {
        self.encode.recv_ping(ping);
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
            self.encode.push_go_away(last_stream_id, reason);
        }

        let fatal = reason != Reason::NO_ERROR;

        if fatal {
            self.encode.close();
        }

        fatal
    }

    pub(crate) fn poll_stream_frame(
        &mut self,
        id: &StreamId,
        pending_window: &mut RecvWindow,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame, StreamError>>> {
        let stream = self.stream_map.get_mut(id).expect(STREAM_MUST_EXIST);

        stream.poll_frame(&mut self.frame_buf, cx).map_ok(|frame| {
            if let Some(bytes) = frame.data_ref() {
                // bytes.len() bounded by SETTINGS_MAX_FRAME_SIZE (24-bit per RFC §6.5.2),
                // so the cast is exact.
                let window = RecvWindow::new(bytes.len() as u32);
                *pending_window += window;

                // Connection credit is batched across bodies by the writer,
                // independently of each stream's consumption threshold.
                self.encode.connection_window_update(window);

                if *pending_window >= self.recv_stream_threshold {
                    let window = mem::replace(pending_window, RecvWindow::ZERO);
                    stream.recv_window_update(window);
                    self.encode.stream_window_update(*id, window);
                }

                // A RequestBody can be polled outside the dispatcher's response queue.
                self.encode.wake();
            }
            frame
        })
    }

    pub(crate) fn is_recv_end_stream(&self, id: &StreamId) -> bool {
        self.stream_map.get(id).expect(STREAM_MUST_EXIST).is_recv_end_stream()
    }

    pub(super) fn try_set_pending_ping(&mut self) -> io::Result<()> {
        self.encode.try_set_pending_ping()
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
    pub(crate) fn recv_setting(&mut self, setting: Settings) -> Result<(), Error> {
        if setting.is_ack() {
            return Ok(());
        }

        if let Some(new_window) = setting.initial_window_size() {
            let new_initial = SendWindow::new(new_window as i32);
            let delta = new_initial - self.send_stream_initial_window;
            self.send_stream_initial_window = new_initial;

            if delta > SendWindow::ZERO {
                for stream in self.stream_map.values() {
                    stream
                        .send_window_check(delta)
                        .map_err(|err| Error::GoAway(err.reason()))?;
                }
            }

            if delta != SendWindow::ZERO {
                self.update_and_wake_send_streams(delta);
            }
        }

        self.encode.recv_setting(setting)
    }

    pub(crate) fn poll_encode(&mut self, write_buf: &mut BytesMut, cx: &mut Context<'_>) -> Poll<bool> {
        self.encode.poll_encode(write_buf, &mut self.recv_connection_window, cx)
    }

    pub(crate) fn send_headers(&mut self, headers: Headers<ResponsePseudo>) {
        self.encode.push_headers(headers);
    }

    pub(crate) fn poll_send_data(
        &mut self,
        id: StreamId,
        data: &mut Bytes,
        end_stream: bool,
        cx: &mut Context<'_>,
    ) -> Poll<Option<()>> {
        // Empty payload bypasses flow control (RFC 9113 §6.9.1: a zero-length
        // frame with END_STREAM set MAY be sent when there is no space left in
        // either flow-control window). No window or waker consultation needed;
        // queue directly and return.
        if data.is_empty() {
            let opt = if !end_stream {
                tracing::warn!("Empty Data frame is not allowed unless it's the last frame of stream");
                None
            } else {
                let payload = mem::take(data);
                self.encode.push_data(id, payload, end_stream);
                Some(())
            };
            return Poll::Ready(opt);
        }

        let stream = self.stream_map.get_mut(&id).expect(STREAM_MUST_EXIST);

        loop {
            let len = data.len();

            let req = SendWindow::from_usize_saturating(len).min(self.encode.max_frame_size());

            let Some(Ok(aval)) = ready!(stream.poll_send_window(req, &mut self.send_connection_window, cx)) else {
                return Poll::Ready(Some(()));
            };

            let aval = aval.as_frame_size();
            let all_consumed = aval == len;

            let payload = if all_consumed {
                mem::take(data)
            } else {
                data.split_to(aval)
            };

            let end_stream = all_consumed && end_stream;

            self.encode.push_data(id, payload, end_stream);

            if end_stream {
                return Poll::Ready(Some(()));
            } else if all_consumed {
                return Poll::Ready(None);
            }
        }
    }

    pub(crate) fn send_trailers(&mut self, id: StreamId, trailers: HeaderMap) {
        self.encode.push_trailers(id, trailers);
    }

    pub(crate) fn send_end_stream(&mut self, id: StreamId) {
        self.encode.push_end_stream(id);
    }

    pub(crate) fn close_write_queue(&mut self) {
        self.encode.close();
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

    pub(crate) fn init(&mut self, settings: Settings) {
        let delta = self.recv_stream_initial_window.saturating_sub(RecvWindow::default());
        self.encode.connection_window_update(delta);
        self.encode.push_settings(settings);
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

#[cfg(test)]
mod tests;
