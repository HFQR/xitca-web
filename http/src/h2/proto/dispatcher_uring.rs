use core::{
    cell::RefCell,
    fmt,
    future::{Future, poll_fn},
    mem,
    pin::{Pin, pin},
    task::{Context, Poll, Waker},
};

use std::{
    collections::{HashMap, VecDeque},
    io,
    rc::Rc,
};

use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
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
    util::futures::Queue,
};

use super::{
    PREFACE, data,
    error::Error,
    go_away::GoAway,
    head,
    headers::{self, ResponsePseudo},
    hpack,
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
    remote_setting: Settings,
    decoder: hpack::Decoder,
    // next_frame_len == 0 is used as maker for waiting for new frame.
    next_frame_len: usize,
    continuation: Option<(headers::Headers, BytesMut)>,
    flow: &'a SharedFlowControl,
    writer_tx: &'a SharedWriterQueue,
}

struct StreamState {
    /// Receive side: body sender + receive window.
    /// `None` after request END_STREAM or trailer.
    body_tx: Option<(RequestBodySender, usize)>,
    /// Send side: flow control for the response body.
    /// `None` until the response task starts streaming, or after it finishes.
    send_flow: Option<StreamControlFlow>,
}

impl StreamState {
    /// Returns `true` if both sides are done and this entry can be removed.
    fn is_empty(&self) -> bool {
        self.body_tx.is_none() && self.send_flow.is_none()
    }
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
}

impl FlowControl {
    /// Remove all state for a stream. Drops the body sender (so `RequestBody`
    /// sees EOF) and wakes any blocked response task so it can exit cleanly.
    fn remove_stream(&mut self, id: StreamId) {
        if let Some(state) = self.stream_map.remove(&id) {
            if let Some(sf) = state.send_flow {
                if let Some(waker) = sf.waker {
                    waker.wake();
                }
            }
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
                    if let Some(waker) = sf.waker.take() {
                        waker.wake();
                    }
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
    Reset(StreamId, Reason),
    WindowUpdate(StreamId, usize),
    Settings,
    /// Echo the 8-byte payload back as a PING ACK (RFC 7540 §6.7).
    Ping([u8; 8]),
    /// Send GOAWAY then close the connection (RFC 7540 §6.8).
    /// Carries the last successfully processed stream ID and the error reason.
    GoAway(StreamId, Reason),
}

struct WriterQueue {
    messages: VecDeque<Message>,
    waker: Option<Waker>,
    closed: bool,
}

type SharedWriterQueue = Rc<RefCell<WriterQueue>>;

impl WriterQueue {
    fn new() -> Self {
        Self {
            messages: VecDeque::new(),
            waker: None,
            closed: false,
        }
    }

    fn push(&mut self, msg: Message) {
        self.messages.push_back(msg);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
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
            if let Message::Data(ref mut d) = msg {
                if d.stream_id() == stream_id {
                    d.set_end_stream(true);
                    if let Some(waker) = self.waker.take() {
                        waker.wake();
                    }
                    return;
                }
            }
        }
        // Fallback: last DATA already consumed by writer. Send a zero-length
        // DATA frame with END_STREAM (9 bytes on the wire, no window cost).
        let mut data = data::Data::new(stream_id, Bytes::new());
        data.set_end_stream(true);
        self.push(Message::Data(data));
    }

    fn close(&mut self) {
        self.closed = true;
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Message>> {
        if let Some(msg) = self.messages.pop_front() {
            Poll::Ready(Some(msg))
        } else if self.closed {
            Poll::Ready(None)
        } else {
            self.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl<'a> DecodeContext<'a> {
    /// Returns `Err(GoAway(PROTOCOL_ERROR))` if `id` refers to an idle stream
    /// (never opened), per RFC 7540 §5.1.
    fn check_not_idle(&self, id: StreamId) -> Result<(), Error> {
        if id > self.last_stream_id {
            Err(Error::GoAway(Reason::PROTOCOL_ERROR))
        } else {
            Ok(())
        }
    }

    fn new(flow: &'a SharedFlowControl, writer_tx: &'a SharedWriterQueue, max_concurrent_streams: usize) -> Self {
        Self {
            remote_setting: Settings::default(),
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
                // RFC 7540 §4.2: frames exceeding our advertised MAX_FRAME_SIZE
                // are a connection error FRAME_SIZE_ERROR.
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

            // RFC 7540 §6.10: while a header block is in progress, the peer
            // MUST NOT send any frame type other than CONTINUATION.  Any
            // other frame on any stream is a connection error PROTOCOL_ERROR.
            if self.continuation.is_some() && head.kind() != head::Kind::Continuation {
                return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
            }

            self.decode_frame(head, frame, on_msg)?;
        }
    }

    fn decode_frame<F>(&mut self, head: head::Head, mut frame: BytesMut, on_msg: &mut F) -> Result<(), Error>
    where
        F: FnMut(Request<RequestExt<RequestBody>>, StreamId),
    {
        match head.kind() {
            head::Kind::Settings => self.handle_settings(head, &frame)?,
            head::Kind::Headers => {
                let (mut headers, mut payload) = headers::Headers::load(head, frame)?;

                let is_end_headers = headers.is_end_headers();

                headers.load_hpack(&mut payload, self.max_header_list_size, &mut self.decoder)?;

                if !is_end_headers {
                    self.continuation = Some((headers, payload));
                    return Ok(());
                }

                let id = headers.stream_id();

                self.handle_header_frame(id, headers, on_msg)?;
            }
            head::Kind::Continuation => {
                let is_end_headers = (head.flag() & 0x4) == 0x4;

                let (mut headers, mut payload) = self.continuation.take().ok_or_else(|| {
                    // CONTINUATION without a preceding HEADERS/PUSH_PROMISE is a
                    // connection error (RFC 7540 §6.10).
                    Error::GoAway(Reason::PROTOCOL_ERROR)
                })?;

                let id = headers.stream_id();

                if id != head.stream_id() {
                    // Stream ID must match the preceding HEADERS frame (RFC 7540 §6.10).
                    return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
                }

                payload.extend_from_slice(&frame);

                if let Err(e) = headers.load_hpack(&mut payload, self.max_header_list_size, &mut self.decoder) {
                    return match e {
                        Error::Hpack(hpack::DecoderError::NeedMore(_)) if !is_end_headers => {
                            self.continuation = Some((headers, payload));
                            Ok(())
                        }
                        // HPACK decode failure is a connection error COMPRESSION_ERROR
                        // (RFC 7540 §4.3).
                        _ => Err(Error::GoAway(Reason::COMPRESSION_ERROR)),
                    };
                }

                self.handle_header_frame(id, headers, on_msg)?;
            }
            head::Kind::Data => {
                let data = data::Data::load(head, frame.freeze())?;
                let is_end = data.is_end_stream();
                let id = data.stream_id();
                let payload = data.into_payload();
                let payload_len = payload.len();

                if !payload.is_empty() {
                    let mut flow = self.flow.borrow_mut();

                    // RFC 7540 §6.9.1: DATA beyond the connection window is a
                    // connection error FLOW_CONTROL_ERROR.
                    if payload_len > flow.recv_connection_window {
                        return Err(Error::GoAway(Reason::FLOW_CONTROL_ERROR));
                    }
                    flow.recv_connection_window -= payload_len;

                    let reset_reason = if let Some(state) = flow.stream_map.get_mut(&id) {
                        if let Some((ref tx, ref mut w)) = state.body_tx {
                            if payload_len > *w {
                                Some(Reason::FLOW_CONTROL_ERROR)
                            } else {
                                *w -= payload_len;
                                if tx.send(Ok(Frame::Data(payload))).is_err() {
                                    Some(Reason::CANCEL)
                                } else {
                                    if is_end {
                                        state.body_tx = None;
                                        if state.is_empty() {
                                            flow.stream_map.remove(&id);
                                        }
                                    }
                                    None
                                }
                            }
                        } else {
                            // body_tx is None → request side already ended; ignore DATA.
                            None
                        }
                    } else {
                        self.check_not_idle(id)?;
                        drop(flow);
                        self.writer_tx
                            .borrow_mut()
                            .push(Message::Reset(id, Reason::STREAM_CLOSED));
                        return Ok(());
                    };

                    if let Some(reason) = reset_reason {
                        flow.remove_stream(id);
                        drop(flow);
                        self.writer_tx.borrow_mut().push(Message::Reset(id, reason));
                        return Ok(());
                    }
                } else if is_end {
                    let mut flow = self.flow.borrow_mut();
                    if let Some(state) = flow.stream_map.get_mut(&id) {
                        state.body_tx = None;
                        if state.is_empty() {
                            flow.stream_map.remove(&id);
                        }
                    }
                }
            }
            head::Kind::WindowUpdate => {
                let window = WindowUpdate::load(head, frame.as_ref())?;

                match (window.size_increment(), window.stream_id()) {
                    // RFC 7540 §6.9: zero increment is a PROTOCOL_ERROR.
                    (0, 0) => return Err(Error::GoAway(Reason::PROTOCOL_ERROR)),
                    (0, id) => {
                        self.flow.borrow_mut().remove_stream(id);
                        self.writer_tx
                            .borrow_mut()
                            .push(Message::Reset(id, Reason::PROTOCOL_ERROR));
                        return Ok(());
                    }
                    (incr, 0) => {
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
                        if let Some(state) = flow.stream_map.get_mut(&id) {
                            if let Some(ref mut sf) = state.send_flow {
                                if sf.window + incr > settings::MAX_INITIAL_WINDOW_SIZE as i64 {
                                    flow.remove_stream(id);
                                    drop(flow);
                                    self.writer_tx
                                        .borrow_mut()
                                        .push(Message::Reset(id, Reason::FLOW_CONTROL_ERROR));
                                } else {
                                    sf.window += incr;
                                    let waker = sf.waker.take();
                                    if flow.send_connection_window > 0 {
                                        if let Some(waker) = waker {
                                            waker.wake();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            head::Kind::Ping => {
                // RFC 7540 §6.7: PING payload MUST be exactly 8 bytes.
                let payload: [u8; 8] = frame
                    .as_ref()
                    .try_into()
                    .map_err(|_| Error::GoAway(Reason::FRAME_SIZE_ERROR))?;
                // Ignore ACKs (flag 0x1); echo everything else back as ACK.
                if head.flag() & 0x1 == 0 {
                    self.writer_tx.borrow_mut().push(Message::Ping(payload));
                }
            }
            head::Kind::Reset => {
                let reset = reset::Reset::load(head, frame.as_ref())?;
                let id = reset.stream_id();
                // RFC 7540 §6.4: RST_STREAM on an idle stream is a
                // connection error PROTOCOL_ERROR.
                self.check_not_idle(id)?;
                // Remove all stream entries and wake any response task
                // that is blocked waiting for window credit, so it can
                // observe the cancellation and exit cleanly.
                self.flow.borrow_mut().remove_stream(id);
            }
            head::Kind::GoAway => {
                let go_away = GoAway::load(frame.as_ref())?;
                // Peer is closing the connection. Log non-zero reasons for
                // diagnostics then close our side — no GOAWAY reply required.
                if go_away.reason() != Reason::NO_ERROR {
                    tracing::warn!(
                        "received GOAWAY with error: {:?} last_stream={:?}",
                        go_away.reason(),
                        go_away.last_stream_id(),
                    );
                }
                return Err(Error::MalformedMessage);
            }
            head::Kind::PushPromise => {
                // RFC 7540 §8.2: clients MUST NOT send PUSH_PROMISE.
                // Receipt is a connection error PROTOCOL_ERROR.
                return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
            }
            _ => {}
        }
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn handle_settings(&mut self, head: head::Head, frame: &BytesMut) -> Result<(), Error> {
        let setting = Settings::load(head, frame)?;

        if setting.is_ack() {
            return Ok(());
        }

        self.remote_setting = setting;
        let mut flow = self.flow.borrow_mut();

        // RFC 7540 §6.9.2: when INITIAL_WINDOW_SIZE changes, apply the
        // delta to every currently open stream's send window.
        if let Some(new_window) = self.remote_setting.initial_window_size() {
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

        if let Some(frame_size) = self.remote_setting.max_frame_size() {
            let frame_size = frame_size as usize;
            flow.max_frame_size = frame_size;
            for state in flow.stream_map.values_mut() {
                if let Some(ref mut sf) = state.send_flow {
                    sf.frame_size = frame_size;
                }
            }
        }

        drop(flow);
        self.writer_tx.borrow_mut().push(Message::Settings);
        Ok(())
    }

    fn handle_header_frame<F>(&mut self, id: StreamId, headers: headers::Headers, on_msg: &mut F) -> Result<(), Error>
    where
        F: FnMut(Request<RequestExt<RequestBody>>, StreamId),
    {
        let is_end_stream = headers.is_end_stream();

        let (pseudo, headers) = headers.into_parts();

        // Check if this is a trailer (stream already has a body sender).
        {
            let mut flow = self.flow.borrow_mut();
            if let Some(state) = flow.stream_map.get_mut(&id) {
                if let Some((tx, _)) = state.body_tx.take() {
                    let _ = tx.send(Ok(Frame::Trailers(Box::new(headers))));
                }
                if state.is_empty() {
                    flow.stream_map.remove(&id);
                }
                return Ok(());
            }
        }

        // New stream.
        // RFC 7540 §5.1.1: client-initiated streams must use odd IDs and must
        // be strictly greater than any previously seen stream ID.
        if !id.is_client_initiated() || id <= self.last_stream_id {
            return Err(Error::GoAway(Reason::PROTOCOL_ERROR));
        }

        self.last_stream_id = id;

        let Some(method) = pseudo.method else {
            // RFC 7540 §8.1.2.3: :method is required.
            self.writer_tx
                .borrow_mut()
                .push(Message::Reset(id, Reason::PROTOCOL_ERROR));
            return Ok(());
        };

        {
            let mut flow = self.flow.borrow_mut();
            // RFC 7540 §5.1.2: refuse new streams beyond the advertised limit.
            if flow.open_streams >= self.max_concurrent_streams {
                drop(flow);
                self.writer_tx
                    .borrow_mut()
                    .push(Message::Reset(id, Reason::REFUSED_STREAM));
                return Ok(());
            }
            flow.open_streams += 1;
        }

        let mut req = Request::new(RequestExt::<()>::default());
        *req.version_mut() = Version::HTTP_2;
        *req.headers_mut() = headers;
        *req.method_mut() = method;

        let (body, tx) = RequestBody::new_pair(id, Rc::clone(self.writer_tx));

        if is_end_stream {
            drop(tx);
        } else {
            self.flow.borrow_mut().stream_map.insert(
                id,
                StreamState {
                    body_tx: Some((tx, settings::DEFAULT_INITIAL_WINDOW_SIZE as usize)),
                    send_flow: None,
                },
            );
        };

        let req = req.map(|ext| ext.map_body(|_| body));

        on_msg(req, id);
        Ok(())
    }
}

async fn read_io(mut buf: BytesMut, io: &impl AsyncBufRead) -> (io::Result<usize>, BytesMut) {
    let len = buf.len();
    let remaining = buf.capacity() - len;
    if remaining < 4096 {
        buf.reserve(4096 - remaining);
    }
    let (res, buf) = io.read(buf.slice(len..)).await;
    (res, buf.into_inner())
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

    fn poll_encode(&mut self, cx: &mut Context<'_>, write_buf: &mut BytesMut) -> Poll<bool> {
        let poll = loop {
            match self.write_queue.borrow_mut().poll_recv(cx) {
                Poll::Ready(Some(msg)) => match msg {
                    Message::Head(headers) => {
                        let mut buf = write_buf.limit(4096);
                        headers.encode(&mut self.encoder, &mut buf);
                    }
                    Message::Trailer(headers) => {
                        let mut buf = write_buf.limit(4096);
                        headers.encode(&mut self.encoder, &mut buf);
                    }
                    Message::Data(mut data) => {
                        data.encode_chunk(write_buf);
                    }
                    Message::Reset(id, reason) => {
                        let reset = Reset::new(id, reason);
                        reset.encode(write_buf);
                    }
                    Message::WindowUpdate(id, size) => {
                        debug_assert!(size > 0, "window update size must not be 0");
                        {
                            let mut f = self.flow.borrow_mut();
                            f.recv_connection_window += size;
                            if let Some(state) = f.stream_map.get_mut(&id) {
                                if let Some((_, ref mut w)) = state.body_tx {
                                    *w += size;
                                }
                            }
                        }
                        self.pending_conn_window += size;
                        // Stream 0 = connection-only replenishment (e.g. closed
                        // stream). Only accumulate into pending_conn_window;
                        // flush_conn_window will emit the frame.
                        if id != StreamId::zero() {
                            let update = WindowUpdate::new(id, size as _);
                            update.encode(write_buf);
                        }
                    }
                    Message::Settings => {
                        let setting = Settings::ack();
                        setting.encode(write_buf);
                    }
                    Message::Ping(payload) => {
                        let head = head::Head::new(head::Kind::Ping, 0x1, StreamId::zero());
                        head.encode(8, write_buf);
                        write_buf.put_slice(&payload);
                    }
                    Message::GoAway(last_stream_id, reason) => {
                        self.pending_conn_window = 0;
                        let go_away = GoAway::new(last_stream_id, reason);
                        go_away.encode(write_buf);
                        break Poll::Ready(true);
                    }
                },
                Poll::Pending if write_buf.is_empty() => break Poll::Pending,
                Poll::Pending => break Poll::Ready(false),
                Poll::Ready(None) => break Poll::Ready(true),
            }
        };

        self.flush_conn_window(write_buf);

        poll
    }

    /// Flush any accumulated connection-level WINDOW_UPDATE into the write buffer.
    #[inline]
    fn flush_conn_window(&mut self, write_buf: &mut BytesMut) {
        let pending = mem::replace(&mut self.pending_conn_window, 0);
        if pending > 0 {
            let update = WindowUpdate::new(StreamId::zero(), pending as _);
            update.encode(write_buf);
        }
    }
}

/// Drive a single response stream: call the service, encode HEADERS, then
/// stream DATA/Trailers with flow-control awareness.
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
            }
        }
    }

    let _guard = StreamGuard { stream_id, flow };

    let res = match service.call(req).await {
        Ok(res) => res,
        Err(e) => {
            error!("service error: {:?}", e);
            writer_tx
                .borrow_mut()
                .push(Message::Reset(stream_id, Reason::INTERNAL_ERROR));
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
        {
            let mut f = flow.borrow_mut();
            let sf = StreamControlFlow {
                window: f.stream_window,
                frame_size: f.max_frame_size,
                waker: None,
            };
            f.stream_map
                .entry(stream_id)
                .or_insert(StreamState {
                    body_tx: None,
                    send_flow: None,
                })
                .send_flow = Some(sf);
        }

        let mut body = pin!(body);

        'body: loop {
            match poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                None => {
                    writer_tx.borrow_mut().push_end_stream(stream_id);
                    break;
                }
                Some(Err(e)) => {
                    error!("body error: {:?}", e);
                    writer_tx
                        .borrow_mut()
                        .push(Message::Reset(stream_id, Reason::INTERNAL_ERROR));
                    break;
                }
                Some(Ok(frame)) => match frame {
                    Frame::Data(mut bytes) => {
                        while !bytes.is_empty() {
                            let len = bytes.len();

                            let aval = poll_fn(|cx| {
                                let mut f = flow.borrow_mut();

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

                                let len = core::cmp::min(len, sf.frame_size);
                                let aval = core::cmp::min(sf.window as usize, f.send_connection_window);
                                let aval = core::cmp::min(aval, len);
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

/// Experimental h2 http layer.
pub async fn run<Io, S, ResB, ResBE>(io: Io, service: &S) -> io::Result<()>
where
    Io: AsyncBufRead + AsyncBufWrite,
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ResB: Stream<Item = Result<Frame, ResBE>>,
    ResBE: fmt::Debug,
{
    let mut read_buf = BytesMut::new();
    let mut write_buf = BytesMut::new();

    read_buf = prefix_check(&io, read_buf).await?;

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
    });
    let mut ctx = DecodeContext::new(&flow, &writer_queue, max_concurrent_streams);
    let mut queue = Queue::new();

    let mut write_task = pin!(async {
        let mut enc = EncodeContext::new(&flow, &writer_queue);

        loop {
            let is_eof = poll_fn(|cx| enc.poll_encode(cx, &mut write_buf)).await;

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
            .await
        {
            SelectOutput::A(SelectOutput::A((res, buf))) => {
                read_buf = buf;
                if res? == 0 {
                    break;
                }

                let res = ctx.try_decode(&mut read_buf, &mut |req, stream_id| {
                    let s = &service;
                    let t = Rc::clone(&writer_queue);
                    let flow = &flow;

                    queue.push(response_task(req, stream_id, s, t, flow));
                });

                let reason = match res {
                    Ok(()) => None,
                    Err(Error::GoAway(reason)) => Some(reason),
                    Err(Error::Hpack(_)) => Some(Reason::COMPRESSION_ERROR),
                    Err(Error::MalformedMessage) => Some(Reason::PROTOCOL_ERROR),
                    Err(Error::Io(e)) => return Err(e),
                };
                if let Some(reason) = reason {
                    writer_queue
                        .borrow_mut()
                        .push(Message::GoAway(ctx.last_stream_id, reason));
                    drop(queue);
                    writer_queue.borrow_mut().close();
                    write_task.as_mut().await?;
                    return Ok(());
                }

                read_task.set(read_io(read_buf, &io));
            }
            SelectOutput::A(SelectOutput::B(_)) => {}
            SelectOutput::B(_) => break,
        }
    }

    drop(queue);
    writer_queue.borrow_mut().close();

    Ok(())
}

#[cold]
#[inline(never)]
async fn prefix_check(io: &impl AsyncBufRead, mut buf: BytesMut) -> io::Result<BytesMut> {
    while buf.len() < PREFACE.len() {
        let (res, b) = read_io(buf, io).await;
        buf = b;
        res?;
    }

    if buf.start_with(PREFACE) {
        buf.advance(PREFACE.len());
        Ok(buf)
    } else {
        // Invalid client preface — connection cannot be established.
        // No GOAWAY is sent because our own preface has not been exchanged yet.
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP/2 client preface",
        ))
    }
}

/// Request body type for Http/2 specifically.
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

impl Stream for RequestBody {
    type Item = Result<Frame, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        let (stream_id, res) = match this.rx.poll_recv(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                let mut stream_id = None;
                // Window accounting applies to DATA bytes only; trailers carry no payload.
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
            // No more items ready: flush accumulated bytes as one WINDOW_UPDATE.
            Poll::Pending => ((this.pending_window > 0).then_some(this.stream_id), Poll::Pending),
            // Stream closed (error or graceful EOF): only replenish the
            // connection receive window (stream 0) since the stream itself
            // no longer needs flow-control capacity.
            poll => ((this.pending_window > 0).then_some(StreamId::zero()), poll),
        };

        if let Some(stream_id) = stream_id {
            let size = mem::replace(&mut this.pending_window, 0);
            this.writer_tx.borrow_mut().push(Message::WindowUpdate(stream_id, size));
        }

        res
    }
}
