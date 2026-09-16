use core::{
    mem,
    task::{Context, Poll, Waker},
};

use std::{collections::VecDeque, io};

use crate::{
    bytes::{BufMut, Bytes, BytesMut},
    http::HeaderMap,
};

use super::super::{
    flow::Error,
    frame::{
        data::Data,
        go_away::GoAway,
        headers::{Headers, ResponsePseudo},
        ping::Ping,
        reason::Reason,
        reset::Reset,
        settings::{self, Settings},
        stream_id::StreamId,
        window_update::WindowUpdate,
    },
    hpack,
    window::{RecvWindow, SendWindow},
};

/// Outbound frames and pending encoding state for one connection.
pub(in crate::h2::proto) struct EncodeContext {
    encoder: hpack::Encoder,
    /// Remote's SETTINGS_MAX_FRAME_SIZE.
    max_frame_size: SendWindow,
    messages: VecDeque<Message>,
    closed: bool,
    /// The peer's latest SETTINGS frame, pending an ACK (CVE-2019-9515).
    /// A well-behaved peer sends one SETTINGS and waits for the ACK before
    /// sending another (RFC 9113 §6.5.3). A second SETTINGS arriving while
    /// one is already pending kills the connection with ENHANCE_YOUR_CALM.
    pending_settings: RemoteSettings,
    /// Accumulated connection-level WINDOW_UPDATE increment. Mutated at push
    /// time by all callers; flushed to a single connection frame after
    /// `poll_encode` drains the queue.
    pending_conn_window: RecvWindow,
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

impl EncodeContext {
    pub(in crate::h2::proto) fn new() -> Self {
        Self {
            encoder: hpack::Encoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE, 4096),
            max_frame_size: SendWindow::from_u32(settings::DEFAULT_MAX_FRAME_SIZE),
            messages: VecDeque::new(),
            closed: false,
            pending_settings: RemoteSettings::default(),
            pending_conn_window: RecvWindow::ZERO,
            keepalive_ping: KeepalivePing::Idle,
            pending_client_ping: None,
            waker: None,
        }
    }

    /// Restore receive credit when its connection WINDOW_UPDATE is encoded.
    pub(in crate::h2::proto) fn poll_encode(
        &mut self,
        write_buf: &mut BytesMut,
        recv_connection_window: &mut RecvWindow,
        cx: &mut Context<'_>,
    ) -> Poll<bool> {
        while let Some(msg) = self.try_recv() {
            match msg {
                Message::Head(headers) => {
                    let frame_size = self.max_frame_size.as_frame_size();
                    let mut cont = headers.encode(&mut self.encoder, &mut write_buf.limit(frame_size));
                    while let Some(c) = cont {
                        cont = c.encode(&mut write_buf.limit(frame_size));
                    }
                }
                Message::Trailer(headers) => {
                    let frame_size = self.max_frame_size.as_frame_size();
                    let mut cont = headers.encode(&mut self.encoder, &mut write_buf.limit(frame_size));
                    while let Some(c) = cont {
                        cont = c.encode(&mut write_buf.limit(frame_size));
                    }
                }
                Message::Data(mut data) => data.encode_chunk(write_buf),
                Message::Reset { stream_id, reason } => Reset::new(stream_id, reason).encode(write_buf),
                Message::WindowUpdate { stream_id, size } => {
                    WindowUpdate::new(stream_id, size.value()).encode(write_buf)
                }
                Message::GoAway { last_stream_id, reason } => {
                    GoAway::new(last_stream_id, reason).encode(write_buf);
                    // GoAway may be graceful (queue stays open to drain in-flight frames)
                    // or forceful (queue closed). The pusher decides via FlowControl::go_away;
                    // we keep draining either way.
                }
                Message::Settings(settings) => settings.encode(write_buf),
            }
        }

        self.pending_settings.encode(&mut self.encoder, write_buf);

        let pending = mem::replace(&mut self.pending_conn_window, RecvWindow::ZERO);
        if pending != RecvWindow::ZERO {
            *recv_connection_window += pending;
            WindowUpdate::new(StreamId::zero(), pending.value()).encode(write_buf);
        }

        // Encode a client PING ACK if one is waiting (take-and-clear).
        if let Some(payload) = self.pending_client_ping.take() {
            Ping::new(payload, true).encode(write_buf);
        }

        self.keepalive_ping.encode(write_buf);

        if !write_buf.is_empty() {
            Poll::Ready(true)
        } else if self.is_closed() {
            Poll::Ready(false)
        } else {
            self.register(cx);
            Poll::Pending
        }
    }

    pub(in crate::h2::proto) fn max_frame_size(&self) -> SendWindow {
        self.max_frame_size
    }

    pub(in crate::h2::proto) fn recv_setting(&mut self, setting: Settings) -> Result<(), Error> {
        if let Some(frame_size) = setting.max_frame_size() {
            self.max_frame_size = SendWindow::new(frame_size as i32);
        }

        // Record the pending ACK. A second SETTINGS before the first is ACKed
        // is a protocol violation and returns ENHANCE_YOUR_CALM (CVE-2019-9515).
        self.pending_settings.try_update(setting)
    }

    pub(in crate::h2::proto) fn recv_ping(&mut self, ping: Ping) {
        if ping.is_ack {
            // ACK for our keepalive PING: return to Idle so `PingPong::tick`
            // does not time out on the next tick.
            self.keepalive_ping = KeepalivePing::Idle;
        } else {
            // Client-initiated PING: always overwrite; we reply with ACK.
            self.pending_client_ping = Some(ping.payload);
        }
    }

    pub(in crate::h2::proto) fn try_set_pending_ping(&mut self) -> io::Result<()> {
        self.keepalive_ping.try_set_pending_ping()
    }

    pub(in crate::h2::proto) fn push_headers(&mut self, headers: Headers<ResponsePseudo>) {
        self.push(Message::Head(headers));
    }

    pub(in crate::h2::proto) fn push_reset(&mut self, stream_id: StreamId, reason: Reason) {
        self.push(Message::Reset { stream_id, reason });
    }

    pub(in crate::h2::proto) fn push_go_away(&mut self, last_stream_id: StreamId, reason: Reason) {
        self.push(Message::GoAway { last_stream_id, reason });
    }

    pub(in crate::h2::proto) fn push_settings(&mut self, settings: Settings) {
        self.push(Message::Settings(settings));
    }

    fn push(&mut self, msg: Message) {
        self.messages.push_back(msg);
    }

    /// Accumulate a connection-level WINDOW_UPDATE into `pending_conn_window`.
    /// Flushed as a single connection frame after `poll_encode` drains the queue.
    pub(in crate::h2::proto) fn connection_window_update(&mut self, size: RecvWindow) {
        self.pending_conn_window += size;
    }

    pub(in crate::h2::proto) fn stream_window_update(&mut self, id: StreamId, size: RecvWindow) {
        if size != RecvWindow::ZERO {
            self.push(Message::WindowUpdate { stream_id: id, size })
        }
    }

    pub(in crate::h2::proto) fn push_data(&mut self, id: StreamId, payload: Bytes, end_stream: bool) {
        let mut data = Data::new(id, payload);
        data.set_end_stream(end_stream);
        self.push(Message::Data(data));
    }

    pub(in crate::h2::proto) fn push_trailers(&mut self, id: StreamId, trailers: HeaderMap) {
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
    pub(in crate::h2::proto) fn push_end_stream(&mut self, stream_id: StreamId) {
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
        self.push_data(stream_id, Bytes::new(), true);
    }

    pub(in crate::h2::proto) fn close(&mut self) {
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

    pub(in crate::h2::proto) fn wake(&mut self) {
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

    /// Encode a SETTINGS ACK if one is pending and stage the peer's HPACK table size.
    /// `Encoder` emits the staged size update at the head of the next header block,
    /// therefore after this ACK on the wire as RFC 7541 §4.2 wants.
    ///
    /// MUST be called after `poll_encode` drained the write queue. The connection
    /// preface pushed by `FlowControl::init` is a queue message and this is the only
    /// writer that would otherwise get ahead of it. No-ops when nothing is pending.
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

enum Message {
    Head(Headers<ResponsePseudo>),
    Data(Data),
    Trailer(Headers<()>),
    Reset { stream_id: StreamId, reason: Reason },
    WindowUpdate { stream_id: StreamId, size: RecvWindow },
    GoAway { last_stream_id: StreamId, reason: Reason },
    Settings(Settings),
}
