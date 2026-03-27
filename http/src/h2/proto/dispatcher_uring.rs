use core::{
    cell::RefCell,
    cmp, fmt,
    future::poll_fn,
    mem,
    pin::{Pin, pin},
    task::{Context, Poll, Waker},
};

use std::{
    collections::{HashMap, VecDeque},
    io,
    rc::Rc,
    time::Duration,
};

use futures_core::stream::Stream;
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    time::Instant,
};
use tracing::error;
use xitca_io::{
    bytes::{Buf, BufMut, BytesMut},
    io_uring::{AsyncBufRead, AsyncBufWrite, BoundedBuf, write_all},
};
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::{
    body::BodySize,
    bytes::Bytes,
    error::BodyError,
    http::{HeaderMap, Request, RequestExt, Response, Version, header::CONTENT_LENGTH},
    util::{
        futures::Queue,
        timer::{KeepAlive, Timeout},
    },
};

use super::{
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
};

/// A single item in a response body stream.
///
/// - `Data` carries a body chunk; the runtime splits it into DATA frames
///   respecting flow-control limits.
/// - `Trailers` carries the trailing HEADERS frame (e.g. gRPC status
///   metadata).  It MUST be the last item in the stream; END_STREAM is
///   set automatically on the encoded frame.
///
/// `Box<HeaderMap>` keeps the enum size close to `Bytes` (≈40 bytes vs
/// ≈88 bytes unboxed), since Data is the overwhelmingly common variant.
pub enum Frame {
    Data(Bytes),
    Trailers(Box<HeaderMap>),
}

struct DecodeContext<'a> {
    max_header_list_size: usize,
    max_concurrent_streams: usize,
    /// Highest stream ID fully accepted by the server. Sent in GOAWAY frames
    /// so the client knows which streams were processed (RFC 7540 §6.8).
    last_stream_id: StreamId,
    decoder: hpack::Decoder,
    next_frame_len: usize,
    continuation: Option<(headers::Headers, BytesMut)>,
    flow: &'a SharedFlowControl,
    writer_tx: &'a SharedWriterQueue,
}

struct StreamState {
    /// Receive side: body sender + receive window.
    /// Closed after request END_STREAM or trailer.
    body_tx: BodyChannel,
    /// Send side: flow control for the response body.
    /// `None` until the response task starts streaming, or after it finishes.
    send_flow: Option<StreamControlFlow>,
}

impl StreamState {
    fn is_empty(&self) -> bool {
        self.body_tx.is_closed() && self.send_flow.is_none()
    }
}

struct BodyChannelInner {
    tx: RequestBodySender,
    /// Remaining bytes the client may send on this stream (RFC 7540 §6.9).
    recv_window: usize,
}

/// Encapsulates the receive-side state for a single request stream:
/// the sender half of the body channel and the remaining flow-control window.
/// Closed internally means the client has finished sending (END_STREAM received
/// or trailer accepted).
struct BodyChannel(Option<BodyChannelInner>);

impl BodyChannel {
    fn open(tx: RequestBodySender, recv_window: usize) -> Self {
        Self(Some(BodyChannelInner { tx, recv_window }))
    }

    fn closed() -> Self {
        Self(None)
    }

    fn is_closed(&self) -> bool {
        self.0.is_none()
    }

    fn close(&mut self) {
        self.0 = None;
    }

    /// Forward a DATA payload. Returns `None` on success, or the `Reason`
    /// for a RST_STREAM if the stream window is exceeded, the receiver has
    /// gone away, or the stream was already half-closed.
    fn forward(&mut self, payload: Bytes) -> Option<Reason> {
        let payload_len = payload.len();
        let inner = self.0.as_mut()?;
        if payload_len > inner.recv_window {
            Some(Reason::FLOW_CONTROL_ERROR)
        } else {
            inner.recv_window -= payload_len;
            // Skip sending empty chunks — zero-length DATA is
            // valid on the wire but needless noise for the service.
            if payload_len > 0 && inner.tx.send(Ok(Frame::Data(payload))).is_err() {
                Some(Reason::CANCEL)
            } else {
                None
            }
        }
    }

    /// Send trailers and close. Returns `false` if already closed
    /// (caller should treat as STREAM_CLOSED).
    fn send_trailer(&mut self, headers: HeaderMap) -> bool {
        let Some(inner) = self.0.take() else {
            return false;
        };
        let _ = inner.tx.send(Ok(Frame::Trailers(Box::new(headers))));
        true
    }

    fn replenish_window(&mut self, size: usize) {
        if let Some(inner) = &mut self.0 {
            inner.recv_window += size;
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

struct FlowControl {
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
    stream_map: HashMap<StreamId, StreamState>,
    /// State of the server-initiated keepalive PING. Replaces the old
    /// `pending_ack` boolean; see `KeepalivePing` for the state transitions.
    keepalive_ping: KeepalivePing,
    /// A client-initiated PING whose ACK we must send. Stored outside the
    /// write queue so queue depth is permanently bounded (CVE-2019-9512).
    /// Always overwritten by the latest client PING; poll_encode takes and
    /// clears it after encoding the ACK.
    pending_client_ping: Option<[u8; 8]>,
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
    /// Remove all state for a stream. Drops the body sender (so `RequestBody`
    /// sees EOF) and wakes any blocked response task so it can exit cleanly.
    fn remove_stream(&mut self, id: StreamId) {
        if let Some(state) = self.stream_map.remove(&id) {
            if let Some(mut sf) = state.send_flow {
                sf.wake();
            }
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
            if let Some(ref mut sf) = state.send_flow {
                sf.window += delta;
                if sf.window > 0 {
                    sf.wake();
                }
            }
        }
    }
}

type SharedFlowControl = RefCell<FlowControl>;

enum Message {
    Head(headers::Headers<ResponsePseudo>),
    Data(data::Data),
    Trailer(headers::Headers<()>),
    Reset { stream_id: StreamId, reason: Reason },
    WindowUpdate { stream_id: StreamId, size: usize },
    GoAway { last_stream_id: StreamId, reason: Reason },
}

struct WriterQueue {
    messages: VecDeque<Message>,
    closed: bool,
}

type SharedWriterQueue = Rc<RefCell<WriterQueue>>;

impl WriterQueue {
    fn new() -> Self {
        Self {
            messages: VecDeque::new(),
            closed: false,
        }
    }

    fn push(&mut self, msg: Message) {
        self.messages.push_back(msg);
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
            if let Message::Data(d) = msg
                && d.stream_id() == stream_id
            {
                d.set_end_stream(true);
                return;
            }
        }

        // Fallback: last DATA already consumed by writer. Send a zero-length
        // DATA frame with END_STREAM (9 bytes on the wire, no window cost).
        self.push_end_stream_cold(stream_id);
    }

    #[cold]
    #[inline(never)]
    fn push_end_stream_cold(&mut self, stream_id: StreamId) {
        let mut data = data::Data::new(stream_id, Bytes::new());
        data.set_end_stream(true);
        self.push(Message::Data(data));
    }

    fn close(&mut self) {
        self.closed = true;
    }

    fn poll_recv(&mut self) -> Poll<Option<Message>> {
        if let Some(msg) = self.messages.pop_front() {
            Poll::Ready(Some(msg))
        } else if self.closed {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

impl<'a> DecodeContext<'a> {
    fn check_not_idle(&self, id: StreamId) -> Result<(), Error> {
        if id > self.last_stream_id {
            Err(Error::GoAway(Reason::PROTOCOL_ERROR))
        } else {
            Ok(())
        }
    }

    fn on_data_payload(&self, id: StreamId, payload: Bytes) -> Result<(), Error> {
        let payload_len = payload.len();
        let mut flow = self.flow.borrow_mut();

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
            Some(state) if !state.body_tx.is_closed() => match state.body_tx.forward(payload) {
                None => return Ok(()),
                Some(reason) => reason,
            },
            // body_tx closed: half-closed (remote); any further DATA is STREAM_CLOSED.
            // stream not in map: fully closed; same error.
            _ => Reason::STREAM_CLOSED,
        };

        flow.remove_stream(id);
        Err(Error::Reset(id, reason, (payload_len > 0).then_some(payload_len)))
    }

    fn new(flow: &'a SharedFlowControl, writer_tx: &'a SharedWriterQueue, max_concurrent_streams: usize) -> Self {
        Self {
            max_header_list_size: settings::DEFAULT_SETTINGS_MAX_HEADER_LIST_SIZE,
            max_concurrent_streams,
            last_stream_id: StreamId::zero(),
            decoder: hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE),
            next_frame_len: 0,
            continuation: None,
            flow,
            writer_tx,
        }
    }

    fn try_decode<F>(&mut self, buf: &mut BytesMut, on_msg: &mut F) -> Result<(), Error>
    where
        F: FnMut(Request<RequestExt<RequestBody>>, StreamId),
    {
        loop {
            if self.next_frame_len == 0 {
                if buf.len() < 3 {
                    return Ok(());
                }
                let payload_len = buf.get_uint(3) as usize;
                if payload_len > settings::DEFAULT_MAX_FRAME_SIZE as usize {
                    return Err(Error::GoAway(Reason::FRAME_SIZE_ERROR));
                }
                self.next_frame_len = payload_len + 6;
            }

            if buf.len() < self.next_frame_len {
                return Ok(());
            }

            let len = mem::replace(&mut self.next_frame_len, 0);
            let mut frame = buf.split_to(len);
            let head = head::Head::parse(&frame);

            // TODO: Make Head::parse auto advance the frame?
            frame.advance(6);
            match self.decode_frame(head, frame, on_msg) {
                Ok(()) => {}
                Err(Error::Reset(id, reason, window)) => {
                    let mut tx = self.writer_tx.borrow_mut();
                    tx.push(Message::Reset { stream_id: id, reason });
                    if let Some(size) = window {
                        tx.push(Message::WindowUpdate {
                            stream_id: StreamId::zero(),
                            size,
                        });
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn decode_frame<F>(&mut self, head: head::Head, frame: BytesMut, on_msg: &mut F) -> Result<(), Error>
    where
        F: FnMut(Request<RequestExt<RequestBody>>, StreamId),
    {
        match (head.kind(), &self.continuation) {
            (head::Kind::Continuation, _) => self.handle_continuation(head, frame, on_msg)?,
            // RFC 7540 §6.10: while a header block is in progress, the peer
            // MUST NOT send any frame type other than CONTINUATION.  Any
            // other frame on any stream is a connection error PROTOCOL_ERROR.
            (_, Some(_)) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
            (head::Kind::Headers, _) => {
                let (headers, payload) = headers::Headers::load(head, frame)?;
                let is_end_headers = headers.is_end_headers();
                self.decode_header_block(headers, payload, is_end_headers, on_msg)?;
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
                self.on_data_payload(id, payload)?;

                if is_end {
                    let mut flow = self.flow.borrow_mut();
                    if let Some(state) = flow.stream_map.get_mut(&id) {
                        state.body_tx.close();
                        if state.is_empty() {
                            flow.stream_map.remove(&id);
                        }
                    }
                }
            }
            (head::Kind::WindowUpdate, _) => {
                let window = WindowUpdate::load(head, frame.as_ref())?;

                match (window.size_increment(), window.stream_id()) {
                    (0, StreamId::ZERO) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
                    (0, id) => {
                        self.check_not_idle(id)?;
                        self.flow.borrow_mut().remove_stream(id);
                        return Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None));
                    }
                    (incr, StreamId::ZERO) => {
                        let incr = incr as usize;
                        let mut flow = self.flow.borrow_mut();

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
                        let mut flow = self.flow.borrow_mut();
                        let window = flow.send_connection_window;
                        if let Some(state) = flow.get_stream_mut(id, self.last_stream_id)? {
                            if let Some(ref mut sf) = state.send_flow {
                                if sf.window + incr > settings::MAX_INITIAL_WINDOW_SIZE as i64 {
                                    flow.remove_stream(id);
                                    drop(flow);
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
            }
            (head::Kind::Ping, _) => {
                let ping = Ping::load(head, frame.as_ref())?;
                if ping.is_ack {
                    // ACK for our keepalive PING: return to Idle so `PingPong::tick`
                    // does not time out on the next tick.
                    self.flow.borrow_mut().keepalive_ping = KeepalivePing::Idle;
                } else {
                    // Client-initiated PING: always overwrite; we reply with ACK.
                    self.flow.borrow_mut().pending_client_ping = Some(ping.payload);
                }
            }
            (head::Kind::Reset, _) => {
                let reset = Reset::load(head, frame.as_ref())?;
                let id = reset.stream_id();
                if id.is_zero() {
                    return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
                }
                self.check_not_idle(id)?;
                let mut flow = self.flow.borrow_mut();
                // A stream is "prematurely" reset if the response task is still
                // running (send_flow is Some). Count these to detect rapid-reset
                // abuse (CVE-2023-44487). Legitimate resets (e.g. client timeout
                // after full request) are included but rare enough not to matter.
                if flow.stream_map.get(&id).and_then(|s| s.send_flow.as_ref()).is_some() {
                    flow.premature_reset_count += 1;
                    if flow.premature_reset_count > PREMATURE_RESET_LIMIT {
                        return Err(Error::GoAway(Reason::ENHANCE_YOUR_CALM));
                    }
                }
                flow.remove_stream(id);
            }
            (head::Kind::GoAway, _) => {
                let go_away = GoAway::load(head.stream_id(), frame.as_ref())?;
                if go_away.reason() == Reason::NO_ERROR {
                    return Err(Error::GoAway(Reason::NO_ERROR));
                }
                tracing::warn!(
                    "received GOAWAY with error: {:?} last_stream={:?}",
                    go_away.reason(),
                    go_away.last_stream_id(),
                );
                return Err(Error::PeerAccused);
            }
            (head::Kind::PushPromise, _) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
            (head::Kind::Settings, _) => self.handle_settings(head, &frame)?,
            _ => {}
        }
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn handle_continuation<F>(&mut self, head: head::Head, frame: BytesMut, on_msg: &mut F) -> Result<(), Error>
    where
        F: FnMut(Request<RequestExt<RequestBody>>, StreamId),
    {
        let is_end_headers = (head.flag() & 0x4) == 0x4;

        // RFC 7540 §6.10: CONTINUATION without a preceding incomplete HEADERS
        // is a connection error PROTOCOL_ERROR
        let (headers, mut payload) = self.continuation.take().ok_or(Error::GoAway(Reason::PROTOCOL_ERROR))?;

        if headers.stream_id() != head.stream_id() {
            return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
        }

        payload.extend_from_slice(&frame);
        self.decode_header_block(headers, payload, is_end_headers, on_msg)
    }

    #[cold]
    #[inline(never)]
    fn handle_settings(&mut self, head: head::Head, frame: &BytesMut) -> Result<(), Error> {
        let setting = Settings::load(head, frame)?;

        if setting.is_ack() {
            return Ok(());
        }

        let mut flow = self.flow.borrow_mut();

        if let Some(new_window) = setting.initial_window_size() {
            let new_window = new_window as i64;
            let delta = new_window - flow.stream_window;
            flow.stream_window = new_window;

            if delta > 0 {
                let overflow = flow
                    .stream_map
                    .values()
                    .filter_map(|s| s.send_flow.as_ref())
                    .any(|sf| sf.window + delta > settings::MAX_INITIAL_WINDOW_SIZE as i64);
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
                if let Some(ref mut sf) = state.send_flow {
                    sf.frame_size = frame_size;
                }
            }
        }

        // Record the pending ACK. A second SETTINGS before the first is ACKed
        // is a protocol violation and returns ENHANCE_YOUR_CALM (CVE-2019-9515).
        flow.remote_settings.try_update(setting)
    }

    fn decode_header_block<F>(
        &mut self,
        mut headers: headers::Headers,
        mut payload: BytesMut,
        is_end_headers: bool,
        on_msg: &mut F,
    ) -> Result<(), Error>
    where
        F: FnMut(Request<RequestExt<RequestBody>>, StreamId),
    {
        if let Err(e) = headers.load_hpack(&mut payload, self.max_header_list_size, &mut self.decoder) {
            return match e {
                // NeedMore on a multi-frame header block is normal; accumulate and wait
                // for CONTINUATION frames (RFC 7540 §6.10).
                Error::Hpack(hpack::DecoderError::NeedMore(_)) if !is_end_headers => {
                    self.continuation = Some((headers, payload));
                    Ok(())
                }
                _ => Err(Error::GoAway(Reason::COMPRESSION_ERROR)),
            };
        }

        if !is_end_headers {
            self.continuation = Some((headers, payload));
            return Ok(());
        }

        let id = headers.stream_id();
        self.handle_header_frame(id, headers, on_msg)
    }

    fn handle_header_frame<F>(&mut self, id: StreamId, headers: headers::Headers, on_msg: &mut F) -> Result<(), Error>
    where
        F: FnMut(Request<RequestExt<RequestBody>>, StreamId),
    {
        let is_end_stream = headers.is_end_stream();

        let (pseudo, headers) = headers.into_parts();

        {
            let mut flow = self.flow.borrow_mut();
            if let Some(state) = flow.stream_map.get_mut(&id) {
                // RFC 7540 §8.1: trailer HEADERS MUST carry END_STREAM.
                if !is_end_stream {
                    state.body_tx.close();
                    drop(flow);
                    return Err(Error::Reset(id, Reason::PROTOCOL_ERROR, None));
                }
                if !state.body_tx.send_trailer(headers) {
                    drop(flow);
                    return Err(Error::Reset(id, Reason::STREAM_CLOSED, None));
                }
                if state.is_empty() {
                    flow.stream_map.remove(&id);
                }
                return Ok(());
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

        let mut flow = self.flow.borrow_mut();
        // RFC 7540 §5.1.2: refuse new streams beyond the advertised limit.
        // Do NOT update last_stream_id: REFUSED_STREAM means no application
        // processing occurred and the client should retry (RFC 7540 §8.1.4).
        if flow.open_streams >= self.max_concurrent_streams {
            drop(flow);
            return Err(Error::Reset(id, Reason::REFUSED_STREAM, None));
        }
        flow.open_streams += 1;

        // Initialize send_flow at stream creation so any WINDOW_UPDATE that
        // arrives before response_task starts its body loop is not lost.
        let sf = StreamControlFlow {
            window: flow.stream_window,
            frame_size: flow.max_frame_size,
            waker: None,
        };

        self.last_stream_id = id;

        let mut req = Request::new(RequestExt::<()>::default());
        *req.version_mut() = Version::HTTP_2;
        *req.headers_mut() = headers;
        *req.method_mut() = method;

        let (body, tx) = RequestBody::new_pair(id, Rc::clone(self.writer_tx));

        if is_end_stream {
            drop(tx);
            flow.stream_map.insert(
                id,
                StreamState {
                    body_tx: BodyChannel::closed(),
                    send_flow: Some(sf),
                },
            );
        } else {
            flow.stream_map.insert(
                id,
                StreamState {
                    body_tx: BodyChannel::open(tx, settings::DEFAULT_INITIAL_WINDOW_SIZE as usize),
                    send_flow: Some(sf),
                },
            );
        }
        drop(flow);

        let req = req.map(|ext| ext.map_body(|_| body));

        on_msg(req, id);
        Ok(())
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

impl ReadBuf {
    fn new(cap: usize) -> Self {
        Self {
            buf: BytesMut::new(),
            cap,
        }
    }
}

async fn read_io(mut rbuf: ReadBuf, io: &impl AsyncBufRead) -> (io::Result<usize>, ReadBuf) {
    if rbuf.buf.len() >= rbuf.cap {
        // Unprocessed data has hit the cap. Yield without issuing a new read
        // until the caller drains the buffer and restarts the task.
        return core::future::pending().await;
    }
    let len = rbuf.buf.len();
    let remaining = rbuf.buf.capacity() - len;
    if remaining < 4096 {
        rbuf.buf.reserve(4096 - remaining);
    }
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

struct StreamControlFlow {
    /// Remaining send window for this stream. Signed because a SETTINGS change
    /// reducing INITIAL_WINDOW_SIZE can drive it negative; the stream must not
    /// send until WINDOW_UPDATE brings it back above zero (RFC 7540 §6.9.2).
    window: i64,
    frame_size: usize,
    waker: Option<Waker>,
}

impl StreamControlFlow {
    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

struct EncodeContext<'a> {
    encoder: hpack::Encoder,
    flow: &'a SharedFlowControl,
    write_queue: &'a SharedWriterQueue,
    /// Accumulated connection-level WINDOW_UPDATE increment, flushed once
    /// before yielding to the IO layer.
    pending_conn_window: usize,
}

impl<'a> EncodeContext<'a> {
    fn new(flow: &'a SharedFlowControl, write_queue: &'a SharedWriterQueue) -> Self {
        Self {
            encoder: hpack::Encoder::new(65535, 4096),
            flow,
            write_queue,
            pending_conn_window: 0,
        }
    }

    fn poll_encode(&mut self, write_buf: &mut BytesMut) -> Poll<bool> {
        let mut flow = self.flow.borrow_mut();

        flow.remote_settings.encode(&mut self.encoder, write_buf);

        let mut queue = self.write_queue.borrow_mut();

        let is_eof = loop {
            match queue.poll_recv() {
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
                    Message::Data(mut data) => {
                        data.encode_chunk(write_buf);
                    }
                    Message::Reset { stream_id, reason } => {
                        let reset = Reset::new(stream_id, reason);
                        reset.encode(write_buf);
                    }
                    Message::WindowUpdate { stream_id, size } => {
                        debug_assert!(size > 0, "window update size must not be 0");

                        flow.recv_connection_window += size;
                        if let Some(state) = flow.stream_map.get_mut(&stream_id) {
                            state.body_tx.replenish_window(size);
                        }

                        self.pending_conn_window += size;
                        // Stream 0 = connection-only replenishment (e.g. closed
                        // stream). Only accumulate into pending_conn_window;
                        // flush_conn_window will emit the frame.
                        if stream_id != StreamId::zero() {
                            let update = WindowUpdate::new(stream_id, size as _);
                            update.encode(write_buf);
                        }
                    }
                    Message::GoAway { last_stream_id, reason } => {
                        self.pending_conn_window = 0;
                        let go_away = GoAway::new(last_stream_id, reason);
                        go_away.encode(write_buf);
                        // Do NOT break with is_eof=true here. The graceful-drain
                        // path needs the write task to keep running after sending
                        // GOAWAY so that in-flight response frames (DATA, HEADERS,
                        // TRAILERS) queued by response tasks are still delivered.
                        // The write task exits naturally when the queue is both
                        // empty and closed (Poll::Ready(None) below).
                        break false;
                    }
                },
                Poll::Pending => break false,
                Poll::Ready(None) => break true,
            }
        };

        self.flush_conn_window(write_buf);

        // Encode a client PING ACK if one is waiting (take-and-clear).
        if let Some(payload) = flow.pending_client_ping.take() {
            let head = head::Head::new(head::Kind::Ping, 0x1, StreamId::zero());
            head.encode(8, write_buf);
            write_buf.put_slice(&payload);
        }

        // Encode our keepalive PING if it is queued but not yet sent, then
        // transition to InFlight so we do not re-send it on the next pass.
        if flow.keepalive_ping == KeepalivePing::Pending {
            let head = head::Head::new(head::Kind::Ping, 0x0, StreamId::zero());
            head.encode(8, write_buf);
            write_buf.put_slice(&[0u8; 8]);
            flow.keepalive_ping = KeepalivePing::InFlight;
        }

        // Return Pending only when there is nothing to write AND the queue is
        // still open. If is_eof is true (queue closed+empty), return Ready even
        // with an empty buffer: poll_recv returns Poll::Ready(None) without
        // storing a waker, so returning Pending here would stall forever.
        if write_buf.is_empty() && !is_eof {
            Poll::Pending
        } else {
            Poll::Ready(is_eof)
        }
    }

    #[inline]
    fn flush_conn_window(&mut self, write_buf: &mut BytesMut) {
        let pending = mem::replace(&mut self.pending_conn_window, 0);
        if pending > 0 {
            let update = WindowUpdate::new(StreamId::zero(), pending as _);
            update.encode(write_buf);
        }
    }
}

async fn response_task<S, ResB, ResBE>(
    req: Request<RequestExt<RequestBody>>,
    stream_id: StreamId,
    service: &S,
    writer_tx: SharedWriterQueue,
    flow: &SharedFlowControl,
) where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ResB: Stream<Item = Result<Frame, ResBE>>,
    ResBE: fmt::Debug,
{
    struct StreamGuard<'a> {
        stream_id: StreamId,
        flow: &'a SharedFlowControl,
    }

    impl Drop for StreamGuard<'_> {
        fn drop(&mut self) {
            let mut flow = self.flow.borrow_mut();
            flow.open_streams -= 1;
            if let Some(state) = flow.stream_map.get_mut(&self.stream_id) {
                state.send_flow = None;
                if state.is_empty() {
                    flow.stream_map.remove(&self.stream_id);
                }
                // Stream was still in the map: it completed normally (not evicted
                // by RST_STREAM). Credit back one premature reset so well-behaved
                // connections never accumulate toward the limit over time.
                flow.premature_reset_count = flow.premature_reset_count.saturating_sub(1);
            }
        }
    }

    let _guard = StreamGuard { stream_id, flow };

    let res = match service.call(req).await {
        Ok(res) => res,
        Err(e) => {
            error!("service error: {:?}", e);
            writer_tx.borrow_mut().push(Message::Reset {
                stream_id,
                reason: Reason::INTERNAL_ERROR,
            });
            return;
        }
    };

    let (mut parts, body) = res.into_parts();

    let size = BodySize::from_stream(&body);

    if let BodySize::Sized(size) = size {
        parts.headers.insert(CONTENT_LENGTH, size.into());
    }

    let pseudo = headers::Pseudo::response(parts.status);
    let mut headers = headers::Headers::new(stream_id, pseudo, parts.headers);

    let has_body = !matches!(size, BodySize::None);
    if !has_body {
        headers.set_end_stream();
    }
    writer_tx.borrow_mut().push(Message::Head(headers));

    if has_body {
        let mut body = pin!(body);

        'body: loop {
            match poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                None => {
                    writer_tx.borrow_mut().push_end_stream(stream_id);
                    break;
                }
                Some(Err(e)) => {
                    error!("body error: {:?}", e);
                    writer_tx.borrow_mut().push(Message::Reset {
                        stream_id,
                        reason: Reason::INTERNAL_ERROR,
                    });
                    break;
                }
                Some(Ok(frame)) => match frame {
                    Frame::Data(mut bytes) => {
                        while !bytes.is_empty() {
                            let len = bytes.len();

                            let aval = poll_fn(|cx| {
                                let mut f = flow.borrow_mut();
                                let f = &mut *f;

                                let Some(state) = f.stream_map.get_mut(&stream_id) else {
                                    return Poll::Ready(None);
                                };
                                let Some(ref mut sf) = state.send_flow else {
                                    return Poll::Ready(None);
                                };

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

                            let chunk = bytes.split_to(aval);
                            let data = data::Data::new(stream_id, chunk);
                            writer_tx.borrow_mut().push(Message::Data(data));
                        }
                    }
                    Frame::Trailers(header_map) => {
                        let trailer = headers::Headers::trailers(stream_id, *header_map);
                        writer_tx.borrow_mut().push(Message::Trailer(trailer));
                        break;
                    }
                },
            }
        }
    }
}

struct PingPong<'a> {
    timer: Pin<&'a mut KeepAlive>,
    flow: &'a SharedFlowControl,
}

impl PingPong<'_> {
    async fn tick(&mut self) -> io::Result<()> {
        self.timer.as_mut().await;
        // Keepalive tick: timeout if the previous PING was never ACKed
        // (write_io stalled or peer silent). Otherwise queue a new PING
        // and reset — write_task sees KeepalivePing::Pending on the next
        // select iteration with no explicit wake() required.
        {
            let mut flow = self.flow.borrow_mut();

            if flow.keepalive_ping != KeepalivePing::Idle {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "h2 ping timeout"));
            }
            flow.keepalive_ping = KeepalivePing::Pending;
        }

        self.timer.as_mut().update(Instant::now() + KEEP_ALIVE);

        Ok(())
    }
}

const KEEP_ALIVE: Duration = Duration::from_secs(10);
/// Maximum number of premature RST_STREAMs before the connection is closed
/// with GOAWAY(ENHANCE_YOUR_CALM). Matches the h2 crate's default.
const PREMATURE_RESET_LIMIT: usize = 100;
/// Hard cap on unprocessed bytes in the read buffer. `read_io` yields
/// `Poll::Pending` when this is reached, preventing unbounded ingest when
/// the write side cannot keep up.
const READ_BUF_CAP: usize = settings::DEFAULT_MAX_FRAME_SIZE as usize * 4; // 64 KiB

pub async fn run<Io, S, ResB, ResBE>(io: Io, service: &S) -> io::Result<()>
where
    Io: AsyncBufRead + AsyncBufWrite,
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ResB: Stream<Item = Result<Frame, ResBE>>,
    ResBE: fmt::Debug,
{
    let mut write_buf = BytesMut::new();
    let mut ka = pin!(KeepAlive::new(Instant::now() + KEEP_ALIVE));

    let buf = prefix_check(&io, BytesMut::new(), ka.as_mut()).await?;
    let mut read_buf = ReadBuf { buf, cap: READ_BUF_CAP };

    let mut settings = settings::Settings::default();
    settings.set_max_concurrent_streams(Some(256));

    settings.encode(&mut write_buf);
    let (res, buf) = write_io(write_buf, &io).await;
    write_buf = buf;
    res?;

    let mut read_task = pin!(read_io(read_buf, &io));

    let writer_queue: SharedWriterQueue = Rc::new(RefCell::new(WriterQueue::new()));

    let recv_initial_window_size = settings
        .initial_window_size()
        .unwrap_or(settings::DEFAULT_INITIAL_WINDOW_SIZE);
    let max_frame_size = settings.max_frame_size().unwrap_or(settings::DEFAULT_MAX_FRAME_SIZE);
    let max_concurrent_streams = settings.max_concurrent_streams().unwrap_or(u32::MAX) as usize;

    let flow = RefCell::new(FlowControl {
        open_streams: 0,
        // Send windows start at RFC 7540 §6.9.2 default (65535) until the
        // peer's SETTINGS_INITIAL_WINDOW_SIZE is received and applied.
        send_connection_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as usize,
        stream_window: settings::DEFAULT_INITIAL_WINDOW_SIZE as i64,
        max_frame_size: max_frame_size as usize,
        recv_connection_window: recv_initial_window_size as usize,
        stream_map: HashMap::new(),
        keepalive_ping: KeepalivePing::Idle,
        pending_client_ping: None,
        remote_settings: RemoteSettings::default(),
        premature_reset_count: 0,
    });
    let mut ctx = DecodeContext::new(&flow, &writer_queue, max_concurrent_streams);
    let mut queue = Queue::new();
    let mut ping_pong = PingPong { timer: ka, flow: &flow };

    let mut write_task = pin!(async {
        let mut enc = EncodeContext::new(&flow, &writer_queue);

        loop {
            let is_eof = poll_fn(|_| enc.poll_encode(&mut write_buf)).await;

            let (res, buf) = write_io(write_buf, &io).await;
            write_buf = buf;
            res?;

            if is_eof {
                return Ok::<_, io::Error>(());
            }
        }
    });

    loop {
        match read_task
            .as_mut()
            .select(queue.next())
            .select(write_task.as_mut())
            .select(ping_pong.tick())
            .await
        {
            SelectOutput::A(SelectOutput::A(SelectOutput::A((res, buf)))) => {
                read_buf = buf;
                if res? == 0 {
                    break;
                }

                let res = ctx.try_decode(&mut read_buf.buf, &mut |req, stream_id| {
                    let t = Rc::clone(&writer_queue);
                    queue.push(response_task(req, stream_id, service, t, &flow));
                });

                let reason = match res {
                    Ok(()) => None,
                    Err(Error::GoAway(reason)) => Some(reason),
                    Err(Error::Hpack(_)) => Some(Reason::COMPRESSION_ERROR),
                    Err(Error::MalformedMessage) => Some(Reason::PROTOCOL_ERROR),
                    Err(Error::Io(e)) => return Err(e),
                    Err(Error::PeerAccused) => break,
                    // StreamError is always handled inside try_decode and never propagates.
                    Err(Error::Reset(..)) => unreachable!(),
                };
                if let Some(reason) = reason {
                    writer_queue.borrow_mut().push(Message::GoAway {
                        last_stream_id: ctx.last_stream_id,
                        reason,
                    });
                    if reason == Reason::NO_ERROR {
                        writer_queue.borrow_mut().close();
                        loop {
                            match queue.next().select(write_task.as_mut()).await {
                                SelectOutput::A(_) => {}
                                SelectOutput::B(res) => {
                                    res?;
                                    return Ok(());
                                }
                            }
                        }
                    } else {
                        drop(queue);
                        writer_queue.borrow_mut().close();
                        write_task.as_mut().await?;
                        return Ok(());
                    }
                }

                read_task.set(read_io(read_buf, &io));
            }
            SelectOutput::A(SelectOutput::A(SelectOutput::B(_))) => {}
            SelectOutput::A(SelectOutput::B(res)) => {
                res?;
                break;
            }
            SelectOutput::B(res) => res?,
        }
    }

    drop(queue);
    writer_queue.borrow_mut().close();

    Ok(())
}

#[cold]
#[inline(never)]
async fn prefix_check(io: &impl AsyncBufRead, buf: BytesMut, timer: Pin<&mut KeepAlive>) -> io::Result<BytesMut> {
    async {
        // No cap during preface: the buffer is tiny and always fully drained.
        let mut rbuf = ReadBuf { buf, cap: usize::MAX };
        while rbuf.buf.len() < PREFACE.len() {
            let (res, b) = read_io(rbuf, io).await;
            rbuf = b;
            res?;
        }
        if rbuf.buf.starts_with(PREFACE) {
            rbuf.buf.advance(PREFACE.len());
            Ok(rbuf.buf)
        } else {
            // No GOAWAY is sent because our own preface has not been exchanged yet.
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid HTTP/2 client preface",
            ))
        }
    }
    .timeout(timer)
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "h2 preface timeout"))?
}

pub struct RequestBody {
    stream_id: StreamId,
    rx: UnboundedReceiver<Result<Frame, BodyError>>,
    writer_tx: SharedWriterQueue,
    /// Bytes consumed but not yet reported back as a WINDOW_UPDATE.
    /// Flushed as a single message when the channel has no more items
    /// ready, batching updates across consecutive chunks.
    pending_window: usize,
}

pub type RequestBodySender = UnboundedSender<Result<Frame, BodyError>>;

impl RequestBody {
    fn new_pair(stream_id: StreamId, writer_tx: SharedWriterQueue) -> (Self, RequestBodySender) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Self {
                stream_id,
                rx,
                writer_tx,
                pending_window: 0,
            },
            tx,
        )
    }
}

impl Drop for RequestBody {
    fn drop(&mut self) {
        // Replenish any bytes consumed but not yet acknowledged. The stream
        // itself may already be gone (e.g. RST_STREAM), so only the connection
        // window is restored (stream 0).
        if self.pending_window > 0 {
            let size = mem::replace(&mut self.pending_window, 0);
            self.writer_tx.borrow_mut().push(Message::WindowUpdate {
                stream_id: StreamId::zero(),
                size,
            });
        }
    }
}

impl Stream for RequestBody {
    type Item = Result<Frame, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        let (stream_id, res) = match this.rx.poll_recv(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                let mut stream_id = None;
                if let Frame::Data(ref bytes) = frame {
                    this.pending_window += bytes.len();
                    // Flush when the remaining window would drop below 25% of the
                    // initial size (i.e. 75% consumed). This mirrors nginx's
                    // threshold, which is widely deployed and clients are tuned
                    // to work well against it. It is more eager than the common
                    // window/2 practice, reducing the chance of the peer stalling
                    // while still batching small chunks effectively.
                    if this.pending_window >= settings::DEFAULT_INITIAL_WINDOW_SIZE as usize * 3 / 4 {
                        stream_id = Some(this.stream_id);
                    }
                }
                (stream_id, Poll::Ready(Some(Ok(frame))))
            }
            Poll::Pending => ((this.pending_window > 0).then_some(this.stream_id), Poll::Pending),
            // Stream closed (error or graceful EOF): only replenish the
            // connection receive window (stream 0) since the stream itself
            // no longer needs flow-control capacity.
            poll => ((this.pending_window > 0).then_some(StreamId::zero()), poll),
        };

        if let Some(stream_id) = stream_id {
            let size = mem::replace(&mut this.pending_window, 0);
            this.writer_tx
                .borrow_mut()
                .push(Message::WindowUpdate { stream_id, size });
        }

        res
    }
}
