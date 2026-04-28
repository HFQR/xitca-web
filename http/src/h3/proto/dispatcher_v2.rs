use core::{
    cell::RefCell, fmt, future::Future, future::poll_fn, marker::PhantomData, net::SocketAddr, pin::Pin, pin::pin,
};
use std::{io::Cursor, rc::Rc};

use quinn::{Connection, ConnectionError, RecvStream, SendStream, VarInt as QVarInt, WriteError};
use tokio::sync::Notify;
use xitca_io::net::QuicStream;
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use super::{
    MAX_HEADER_BLOCK_BYTES,
    coding::Encode,
    frame::{Frame, FrameError, FrameType, PayloadLen, SettingId, Settings},
    headers::Header,
    qpack::{Decoder, DecoderError, Encoder, EncoderError, HeaderAck, encode_stateless},
    stream::StreamType,
    varint::{BufMutExt, VarInt},
};

use crate::{
    body::{Body, Frame as BodyFrame},
    bytes::{Buf, Bytes, BytesMut},
    error::HttpServiceError,
    h3::{body_v2::RequestBodyV2, error::Error},
    http::{Extension, HeaderMap, Request, RequestExt, Response, header},
    util::futures::Queue,
};

// RFC 9114 §8.1 connection error codes (subset).
mod h3_code {
    pub(super) const GENERAL_PROTOCOL_ERROR: u64 = 0x101;
    pub(super) const INTERNAL_ERROR: u64 = 0x102;
    pub(super) const STREAM_CREATION_ERROR: u64 = 0x103;
    pub(super) const CLOSED_CRITICAL_STREAM: u64 = 0x104;
    pub(super) const FRAME_UNEXPECTED: u64 = 0x105;
    pub(super) const FRAME_ERROR: u64 = 0x106;
    pub(super) const ID_ERROR: u64 = 0x108;
    pub(super) const MISSING_SETTINGS: u64 = 0x10A;
    pub(super) const REQUEST_INCOMPLETE: u64 = 0x10D;
    pub(super) const MESSAGE_ERROR: u64 = 0x10E;
    pub(super) const QPACK_DECOMPRESSION_FAILED: u64 = 0x200;
    pub(super) const QPACK_ENCODER_STREAM_ERROR: u64 = 0x201;
    pub(super) const QPACK_DECODER_STREAM_ERROR: u64 = 0x202;
}

/// Per-connection state shared between the dispatcher's select branches.
#[derive(Default)]
struct SharedState {
    peer_settings: Option<Settings>,
    peer_control_seen: bool,
    peer_encoder_seen: bool,
    peer_decoder_seen: bool,
    /// Last GOAWAY id received from peer. Subsequent GOAWAYs MUST be
    /// non-increasing (RFC 9114 §5.2).
    peer_goaway: Option<u64>,
    /// Last MAX_PUSH_ID received from peer. Subsequent values MUST be
    /// non-decreasing (RFC 9114 §7.2.7).
    peer_max_push_id: Option<u64>,
}

/// Maximum dynamic table size (in bytes) we're willing to accept from the
/// peer's QPACK encoder. Advertised via `QPACK_MAX_TABLE_CAPACITY` and used
/// to size our local `Decoder`'s table.
const LOCAL_QPACK_MAX_TABLE_CAPACITY: u64 = 4096;

/// QPACK encoder/decoder runtime state shared across the dispatcher's tasks.
struct QpackBus {
    /// Decodes peer-encoded header blocks and processes peer encoder-stream
    /// instructions.
    decoder: RefCell<Decoder>,
    /// Encodes our outbound header blocks and processes peer decoder-stream
    /// instructions (Section Ack / Stream Cancel / Insert Count Increment).
    encoder: RefCell<Encoder>,
    /// Bytes pending to send on our outgoing DECODER stream (Insert Count
    /// Increment, Section Ack, Stream Cancel).
    dec_out: RefCell<BytesMut>,
    /// Bytes pending to send on our outgoing ENCODER stream (table inserts,
    /// dynamic-table-size updates, duplicates).
    enc_out: RefCell<BytesMut>,
    /// Wakes the long-lived DECODER-stream writer when new bytes are queued.
    dec_out_notify: Notify,
    /// Wakes the long-lived ENCODER-stream writer when new bytes are queued.
    enc_out_notify: Notify,
}

impl QpackBus {
    fn new() -> Self {
        let mut decoder = Decoder::new();
        // Configure our decoder to accept up to LOCAL_QPACK_MAX_TABLE_CAPACITY
        // bytes of dynamic table state from peer. Failure here would mean the
        // hardcoded constant exceeds the implementation's max, which can't
        // happen at the value we picked.
        let _ = decoder.set_max_table_size(LOCAL_QPACK_MAX_TABLE_CAPACITY as usize);
        let _ = decoder.set_max_blocked_streams(0);
        Self {
            decoder: RefCell::new(decoder),
            encoder: RefCell::new(Encoder::new()),
            dec_out: RefCell::new(BytesMut::new()),
            enc_out: RefCell::new(BytesMut::new()),
            dec_out_notify: Notify::new(),
            enc_out_notify: Notify::new(),
        }
    }

    fn push_dec(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.dec_out.borrow_mut().extend_from_slice(bytes);
        self.dec_out_notify.notify_one();
    }

    fn push_enc(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.enc_out.borrow_mut().extend_from_slice(bytes);
        self.enc_out_notify.notify_one();
    }
}

/// Connection-scoped state. Shared as `Rc<ConnState>` across the accept
/// loop, the peer-stream handlers, and the per-request handlers.
struct ConnState {
    shared: RefCell<SharedState>,
    qpack: QpackBus,
}

type State = Rc<ConnState>;

/// Boxed future type for entries in the dispatcher's `uni_queue`. Lets us
/// mix `handle_peer_uni_stream` with the long-lived QPACK stream writers in
/// the same `FuturesUnordered`.
type UniFut = Pin<Box<dyn Future<Output = Result<(), ProtoError>>>>;

#[derive(Debug)]
struct ProtoError {
    code: u64,
    reason: String,
}

impl ProtoError {
    fn new(code: u64, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }
}

impl From<ConnectionError> for ProtoError {
    fn from(e: ConnectionError) -> Self {
        Self::new(h3_code::INTERNAL_ERROR, format!("connection: {e:?}"))
    }
}

/// Outcome of a per-request future. Connection-fatal outcomes trigger
/// `quinn::Connection::close`; non-fatal ones just get logged.
enum ReqOutcome<SE, BE> {
    /// Stream-scoped error — the request is already reset (or finished as well
    /// as possible); nothing more to do beyond logging.
    Stream(Error<SE, BE>),
    /// Connection-scoped error — close the whole connection.
    Proto(ProtoError),
}

/// Result of `read_headers_frame`. Distinguishes errors that should reset only
/// the request stream from errors that must close the whole connection.
enum HeadersReadError {
    /// Peer closed (or a transport error occurred) before a complete HEADERS
    /// frame arrived. RFC 9114 §4.1.2 classifies this as a stream error.
    StreamIncomplete(String),
    /// Frame layer or protocol violation that requires a connection error.
    Connection(ProtoError),
}

/// Http/3 dispatcher
pub(crate) struct Dispatcher<'a, S, ReqB> {
    io: QuicStream,
    addr: SocketAddr,
    service: &'a S,
    _req_body: PhantomData<ReqB>,
}

impl<'a, S, ReqB, ResB, BE> Dispatcher<'a, S, ReqB>
where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ResB: Body<Data = Bytes, Error = BE>,
    BE: fmt::Debug,
    ReqB: From<RequestBodyV2>,
{
    pub(crate) fn new(io: QuicStream, addr: SocketAddr, service: &'a S) -> Self {
        Self {
            io,
            addr,
            service,
            _req_body: PhantomData,
        }
    }

    pub(crate) async fn run(self) -> Result<(), Error<S::Error, BE>> {
        let conn = self.io.connecting().await?;
        let addr = self.addr;
        let service = self.service;

        let state: State = Rc::new(ConnState {
            shared: RefCell::new(SharedState::default()),
            qpack: QpackBus::new(),
        });

        // Open and prime our 3 outgoing unidirectional streams. The control
        // stream also carries our initial SETTINGS frame, and is held for the
        // lifetime of the connection so we can emit GOAWAY at shutdown. The
        // QPACK encoder/decoder send halves are moved into long-lived writer
        // futures owned by `uni_queue`.
        let mut control = open_control_stream(&conn).await.map_err(into_err)?;
        let encoder_send = open_typed_uni(&conn, StreamType::ENCODER).await.map_err(into_err)?;
        let decoder_send = open_typed_uni(&conn, StreamType::DECODER).await.map_err(into_err)?;

        let mut uni_queue: Queue<UniFut> = Queue::new();
        uni_queue.push(Box::pin(qpack_stream_writer(
            state.clone(),
            encoder_send,
            QpackOutKind::Encoder,
        )));
        uni_queue.push(Box::pin(qpack_stream_writer(
            state.clone(),
            decoder_send,
            QpackOutKind::Decoder,
        )));

        let mut req_queue = Queue::new();

        let mut shutdown: Option<ProtoError> = None;
        // Largest bidi-client stream id we've accepted. Used at shutdown to
        // emit GOAWAY(largest + 4), promising not to process anything past it.
        let mut largest_accepted_request: Option<u64> = None;

        let outcome = loop {
            let event = conn
                .accept_bi()
                .select(conn.accept_uni())
                .select(uni_queue.next())
                .select(req_queue.next())
                .await;

            match event {
                // accept_bi
                SelectOutput::A(SelectOutput::A(SelectOutput::A(Ok((send, recv))))) => {
                    let id: u64 = recv.id().into();
                    largest_accepted_request = Some(largest_accepted_request.map_or(id, |cur| cur.max(id)));
                    req_queue.push(handle_request_stream(send, recv, addr, service, state.clone()));
                }
                SelectOutput::A(SelectOutput::A(SelectOutput::A(Err(e)))) => {
                    break Err(into_err(e));
                }
                // accept_uni
                SelectOutput::A(SelectOutput::A(SelectOutput::B(Ok(recv)))) => {
                    uni_queue.push(Box::pin(handle_peer_uni_stream(recv, state.clone())));
                }
                SelectOutput::A(SelectOutput::A(SelectOutput::B(Err(e)))) => {
                    break Err(into_err(e));
                }
                // uni_queue completion
                SelectOutput::A(SelectOutput::B(Ok(()))) => {}
                SelectOutput::A(SelectOutput::B(Err(e))) => {
                    shutdown = Some(e);
                    break Ok(());
                }
                // req_queue completion
                SelectOutput::B(Ok(())) => {}
                SelectOutput::B(Err(ReqOutcome::Stream(e))) => {
                    HttpServiceError::from(e).log("h3_dispatcher_v2");
                }
                SelectOutput::B(Err(ReqOutcome::Proto(e))) => {
                    shutdown = Some(e);
                    break Ok(());
                }
            }
        };

        // Drain in-flight futures before exiting so their shared state drops cleanly.
        while !uni_queue.is_empty() {
            let _ = uni_queue.next2().await;
        }
        while !req_queue.is_empty() {
            let _ = req_queue.next2().await;
        }

        if let Some(e) = shutdown {
            // RFC 9114 §5.2: announce the lowest stream id we won't process
            // (largest accepted + 4 for bidi-client) before closing.
            let goaway_id = largest_accepted_request.map_or(0, |id| id.saturating_add(4));
            let _ = send_goaway(&mut control, goaway_id).await;

            let code = QVarInt::from_u64(e.code).unwrap_or(QVarInt::from_u32(0));
            conn.close(code, e.reason.as_bytes());
            return Err(Error::Proto {
                code: e.code,
                reason: e.reason,
            });
        }

        outcome
    }
}

async fn send_goaway(control: &mut SendStream, stream_id: u64) -> Result<(), ConnectionError> {
    let id = VarInt::from_u64(stream_id).unwrap_or(VarInt::from_u32(0));
    let mut buf = BytesMut::with_capacity(VarInt::MAX_SIZE * 3);
    Frame::<Bytes>::Goaway(id).encode(&mut buf);
    write_all_conn(control, &buf).await
}

fn into_err<S, B>(e: ConnectionError) -> Error<S, B> {
    Error::QuinnConnection(e)
}

// ---------------------------------------------------------------------------
// Unidirectional stream handling
// ---------------------------------------------------------------------------

async fn open_control_stream(conn: &Connection) -> Result<SendStream, ConnectionError> {
    let mut send = conn.open_uni().await?;

    let mut buf = BytesMut::with_capacity(StreamType::MAX_ENCODED_SIZE + Settings::MAX_ENCODED_SIZE + 4);
    StreamType::CONTROL.encode(&mut buf);
    Frame::<Bytes>::Settings(local_settings()).encode(&mut buf);
    write_grease_frame(&mut buf);

    write_all_conn(&mut send, &buf).await?;
    Ok(send)
}

/// SETTINGS we advertise to the peer.
///
/// - `MAX_HEADER_LIST_SIZE` bounds the size of header (and trailer) blocks
///   the peer may send; we enforce it on QPACK decode via `MAX_HEADER_BLOCK_BYTES`.
/// - `QPACK_MAX_TABLE_CAPACITY = 0` and `QPACK_MAX_BLOCKED_STREAMS = 0`
///   declare we run a stateless decoder; the peer MUST NOT reference the
///   dynamic table.
/// - One GREASE setting (`0x1f*N + 0x21`) is included per RFC 9114 §7.2.4.1
///   to exercise the peer's "ignore unknown settings" path.
fn local_settings() -> Settings {
    let mut s = Settings::default();
    // unwrap is safe — four distinct ids fit comfortably in the SETTINGS_LEN-sized array.
    s.insert(SettingId::MAX_HEADER_LIST_SIZE, MAX_HEADER_BLOCK_BYTES)
        .unwrap();
    s.insert(SettingId::QPACK_MAX_TABLE_CAPACITY, LOCAL_QPACK_MAX_TABLE_CAPACITY)
        .unwrap();
    // We accept dynamic-table refs but do not (yet) tolerate streams that
    // would block waiting for inserts: the peer must ensure all referenced
    // entries are already in our table by the time a request arrives.
    s.insert(SettingId::QPACK_MAX_BLOCKED_STREAMS, 0).unwrap();
    // Reserved setting id 0x1f*0 + 0x21 = 0x21 with arbitrary value.
    s.insert(SettingId(0x21), 0).unwrap();
    s
}

/// Append a GREASE frame (reserved type `0x1f*N + 0x21`, empty payload) to
/// `buf`. RFC 9114 §7.2.8: receivers MUST treat unknown types as no-ops, so
/// emitting GREASE keeps that ignore-path warm.
fn write_grease_frame(buf: &mut BytesMut) {
    buf.write_var(0x21); // frame type
    buf.write_var(0); // length
}

async fn open_typed_uni(conn: &Connection, ty: StreamType) -> Result<SendStream, ConnectionError> {
    let mut send = conn.open_uni().await?;
    let mut buf = BytesMut::with_capacity(StreamType::MAX_ENCODED_SIZE);
    ty.encode(&mut buf);
    write_all_conn(&mut send, &buf).await?;
    Ok(send)
}

async fn write_all_conn(send: &mut SendStream, buf: &[u8]) -> Result<(), ConnectionError> {
    send.write_all(buf).await.map_err(|e| match e {
        WriteError::ConnectionLost(c) => c,
        _ => ConnectionError::LocallyClosed,
    })
}

/// Identifies which of our outgoing QPACK streams a writer task drives.
#[derive(Copy, Clone)]
enum QpackOutKind {
    Encoder,
    Decoder,
}

/// Long-lived future that owns one of our outgoing QPACK SendStreams and
/// drains pending bytes pushed by other tasks. One instance per stream —
/// runs until the underlying stream errors out (which propagates as a
/// connection-level error).
async fn qpack_stream_writer(state: State, mut send: SendStream, kind: QpackOutKind) -> Result<(), ProtoError> {
    loop {
        let notify = match kind {
            QpackOutKind::Encoder => &state.qpack.enc_out_notify,
            QpackOutKind::Decoder => &state.qpack.dec_out_notify,
        };
        notify.notified().await;
        loop {
            let chunk = {
                let mut pending = match kind {
                    QpackOutKind::Encoder => state.qpack.enc_out.borrow_mut(),
                    QpackOutKind::Decoder => state.qpack.dec_out.borrow_mut(),
                };
                if pending.is_empty() {
                    break;
                }
                pending.split().freeze()
            };
            send.write_all(&chunk).await.map_err(|e| match e {
                WriteError::ConnectionLost(c) => ProtoError::from(c),
                _ => ProtoError::new(h3_code::INTERNAL_ERROR, "qpack stream write"),
            })?;
        }
    }
}

async fn handle_peer_uni_stream(mut recv: RecvStream, state: State) -> Result<(), ProtoError> {
    let ty = read_stream_type(&mut recv).await?;

    match ty {
        StreamType::CONTROL => {
            mark_unique(&state, |s| &mut s.peer_control_seen, "control")?;
            run_peer_control_stream(recv, state).await
        }
        StreamType::ENCODER => {
            mark_unique(&state, |s| &mut s.peer_encoder_seen, "qpack encoder")?;
            run_peer_encoder_stream(recv, state).await
        }
        StreamType::DECODER => {
            mark_unique(&state, |s| &mut s.peer_decoder_seen, "qpack decoder")?;
            run_peer_decoder_stream(recv, state).await
        }
        StreamType::PUSH => Err(ProtoError::new(
            h3_code::STREAM_CREATION_ERROR,
            "server received PUSH stream from peer",
        )),
        _unknown => {
            let _ = recv.stop(QVarInt::from_u64(h3_code::STREAM_CREATION_ERROR).unwrap());
            Ok(())
        }
    }
}

fn mark_unique(
    state: &State,
    field: impl FnOnce(&mut SharedState) -> &mut bool,
    name: &'static str,
) -> Result<(), ProtoError> {
    let mut s = state.shared.borrow_mut();
    let seen = field(&mut s);
    if *seen {
        Err(ProtoError::new(
            h3_code::STREAM_CREATION_ERROR,
            format!("received second {name} stream from peer"),
        ))
    } else {
        *seen = true;
        Ok(())
    }
}

async fn read_stream_type(recv: &mut RecvStream) -> Result<StreamType, ProtoError> {
    use crate::h3::proto::{coding::Decode, varint::VarInt};

    let mut first = [0u8; 1];
    recv.read_exact(&mut first)
        .await
        .map_err(|_| ProtoError::new(h3_code::STREAM_CREATION_ERROR, "stream ended before stream type"))?;

    let total = VarInt::encoded_size(first[0]);
    let mut buf = [0u8; 8];
    buf[0] = first[0];
    if total > 1 {
        recv.read_exact(&mut buf[1..total])
            .await
            .map_err(|_| ProtoError::new(h3_code::STREAM_CREATION_ERROR, "incomplete stream type varint"))?;
    }

    let mut slice: &[u8] = &buf[..total];
    StreamType::decode(&mut slice)
        .map_err(|_| ProtoError::new(h3_code::GENERAL_PROTOCOL_ERROR, "malformed stream type"))
}

async fn run_peer_control_stream(mut recv: RecvStream, state: State) -> Result<(), ProtoError> {
    let mut buf = BytesMut::with_capacity(4096);
    let mut first_frame = true;

    loop {
        let (res, consumed) = {
            let mut cursor = Cursor::new(&buf[..]);
            let res = Frame::<PayloadLen>::decode(&mut cursor);
            (res, cursor.position() as usize)
        };

        match res {
            Ok(frame) => {
                buf.advance(consumed);

                if first_frame {
                    if !matches!(frame, Frame::Settings(_)) {
                        return Err(ProtoError::new(
                            h3_code::MISSING_SETTINGS,
                            "first control stream frame was not SETTINGS",
                        ));
                    }
                    first_frame = false;
                }

                handle_control_frame(frame, &state)?;
            }
            Err(FrameError::Incomplete(_)) => match recv.read_chunk(64 * 1024, true).await {
                Ok(Some(chunk)) => buf.extend_from_slice(&chunk.bytes),
                Ok(None) => {
                    return Err(ProtoError::new(
                        h3_code::CLOSED_CRITICAL_STREAM,
                        "peer closed control stream",
                    ));
                }
                Err(e) => {
                    return Err(ProtoError::new(h3_code::INTERNAL_ERROR, format!("control read: {e:?}")));
                }
            },
            Err(FrameError::UnknownFrame(_)) => {
                buf.advance(consumed);
            }
            Err(FrameError::UnsupportedFrame(_)) => {
                return Err(ProtoError::new(
                    h3_code::FRAME_UNEXPECTED,
                    "h2-only frame on control stream",
                ));
            }
            Err(e) => {
                return Err(ProtoError::new(h3_code::FRAME_ERROR, format!("{e:?}")));
            }
        }
    }
}

fn handle_control_frame(frame: Frame<PayloadLen>, state: &State) -> Result<(), ProtoError> {
    match frame {
        Frame::Settings(s) => {
            let mut state = state.shared.borrow_mut();
            if state.peer_settings.is_some() {
                return Err(ProtoError::new(
                    h3_code::FRAME_UNEXPECTED,
                    "received second SETTINGS on control stream",
                ));
            }
            state.peer_settings = Some(s);
            Ok(())
        }
        Frame::Goaway(id) => {
            //= https://www.rfc-editor.org/rfc/rfc9114#section-5.2
            //# An endpoint MAY send multiple GOAWAY frames indicating different
            //# identifiers, but the identifier in each frame MUST NOT be greater
            //# than the identifier in any previous frame, since clients might
            //# already have retried unprocessed requests on another HTTP
            //# connection.
            let id = id.into_inner();
            let mut s = state.shared.borrow_mut();
            if let Some(prev) = s.peer_goaway {
                if id > prev {
                    return Err(ProtoError::new(
                        h3_code::ID_ERROR,
                        format!("peer GOAWAY id {id} > previous {prev}"),
                    ));
                }
            }
            s.peer_goaway = Some(id);
            Ok(())
        }
        Frame::MaxPushId(id) => {
            //= https://www.rfc-editor.org/rfc/rfc9114#section-7.2.7
            //# A MAX_PUSH_ID frame cannot reduce the maximum push ID; receipt
            //# of a MAX_PUSH_ID frame that contains a smaller value than
            //# previously received MUST be treated as a connection error of
            //# type H3_ID_ERROR.
            let id: u64 = VarInt::from(id).into_inner();
            let mut s = state.shared.borrow_mut();
            if let Some(prev) = s.peer_max_push_id {
                if id < prev {
                    return Err(ProtoError::new(
                        h3_code::ID_ERROR,
                        format!("peer MAX_PUSH_ID {id} < previous {prev}"),
                    ));
                }
            }
            s.peer_max_push_id = Some(id);
            Ok(())
        }
        //= https://www.rfc-editor.org/rfc/rfc9114#section-7.2.3
        //# If a CANCEL_PUSH frame is received that references a push ID
        //# greater than currently allowed on the connection, this MUST be
        //# treated as a connection error of type H3_ID_ERROR.
        Frame::CancelPush(id) => {
            let id: u64 = VarInt::from(id).into_inner();
            let s = state.shared.borrow();
            match s.peer_max_push_id {
                None => Err(ProtoError::new(
                    h3_code::ID_ERROR,
                    "CANCEL_PUSH received before any MAX_PUSH_ID",
                )),
                Some(max) if id >= max => Err(ProtoError::new(
                    h3_code::ID_ERROR,
                    format!("CANCEL_PUSH id {id} >= MAX_PUSH_ID {max}"),
                )),
                Some(_) => Ok(()),
            }
        }
        Frame::PushPromise(_) => Err(ProtoError::new(
            h3_code::FRAME_UNEXPECTED,
            "PUSH_PROMISE on control stream",
        )),
        Frame::Data(_) | Frame::Headers(_) => Err(ProtoError::new(
            h3_code::FRAME_UNEXPECTED,
            "request/response frame on control stream",
        )),
    }
}

/// Process the peer's QPACK ENCODER stream (their encoder → our decoder).
/// Inserts and dynamic-table-size updates from the peer are fed into our
/// `Decoder`. After every successful batch, an Insert Count Increment is
/// queued onto our outgoing DECODER stream (RFC 9204 §3).
async fn run_peer_encoder_stream(mut recv: RecvStream, state: State) -> Result<(), ProtoError> {
    let mut buf = BytesMut::with_capacity(4096);
    loop {
        match recv.read_chunk(64 * 1024, true).await {
            Ok(Some(chunk)) => buf.extend_from_slice(&chunk.bytes),
            Ok(None) => {
                return Err(ProtoError::new(
                    h3_code::CLOSED_CRITICAL_STREAM,
                    "peer closed qpack encoder stream",
                ));
            }
            Err(e) => {
                return Err(ProtoError::new(
                    h3_code::INTERNAL_ERROR,
                    format!("qpack encoder read: {e:?}"),
                ));
            }
        }

        let mut tmp = BytesMut::new();
        let consumed = {
            let mut decoder = state.qpack.decoder.borrow_mut();
            let mut cursor = Cursor::new(&buf[..]);
            match decoder.on_encoder_recv(&mut cursor, &mut tmp) {
                Ok(_) => cursor.position() as usize,
                Err(DecoderError::UnexpectedEnd) => cursor.position() as usize,
                Err(e) => {
                    return Err(ProtoError::new(
                        h3_code::QPACK_ENCODER_STREAM_ERROR,
                        format!("qpack encoder stream: {e:?}"),
                    ));
                }
            }
        };
        buf.advance(consumed);
        if !tmp.is_empty() {
            state.qpack.push_dec(&tmp);
        }
    }
}

/// Process the peer's QPACK DECODER stream (their decoder → our encoder).
/// Carries Section Acks, Stream Cancels, and Insert Count Increments that
/// drive eviction and tracking on our outgoing encoder.
async fn run_peer_decoder_stream(mut recv: RecvStream, state: State) -> Result<(), ProtoError> {
    let mut buf = BytesMut::with_capacity(4096);
    loop {
        match recv.read_chunk(64 * 1024, true).await {
            Ok(Some(chunk)) => buf.extend_from_slice(&chunk.bytes),
            Ok(None) => {
                return Err(ProtoError::new(
                    h3_code::CLOSED_CRITICAL_STREAM,
                    "peer closed qpack decoder stream",
                ));
            }
            Err(e) => {
                return Err(ProtoError::new(
                    h3_code::INTERNAL_ERROR,
                    format!("qpack decoder read: {e:?}"),
                ));
            }
        }

        let consumed = {
            let mut encoder = state.qpack.encoder.borrow_mut();
            let mut cursor = Cursor::new(&buf[..]);
            match encoder.on_decoder_recv(&mut cursor) {
                Ok(()) => cursor.position() as usize,
                Err(EncoderError::InvalidInteger(_)) => cursor.position() as usize,
                Err(e) => {
                    return Err(ProtoError::new(
                        h3_code::QPACK_DECODER_STREAM_ERROR,
                        format!("qpack decoder stream: {e:?}"),
                    ));
                }
            }
        };
        buf.advance(consumed);
    }
}

// ---------------------------------------------------------------------------
// Bidirectional (request) stream handling
// ---------------------------------------------------------------------------

async fn handle_request_stream<S, ReqB, ResB, BE>(
    mut send: SendStream,
    mut recv: RecvStream,
    addr: SocketAddr,
    service: &S,
    state: State,
) -> Result<(), ReqOutcome<S::Error, BE>>
where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    ResB: Body<Data = Bytes, Error = BE>,
    ReqB: From<RequestBodyV2>,
{
    let stream_id: u64 = recv.id().into();
    // Read the opening HEADERS frame. Truncation before HEADERS is a stream
    // error; frame-layer violations are connection errors.
    let (block, leftover) = match read_headers_frame(&mut recv).await {
        Ok(x) => x,
        Err(HeadersReadError::StreamIncomplete(reason)) => {
            reset_stream(&mut send, &mut recv, h3_code::REQUEST_INCOMPLETE);
            return Err(ReqOutcome::Stream(Error::Proto {
                code: h3_code::REQUEST_INCOMPLETE,
                reason,
            }));
        }
        Err(HeadersReadError::Connection(e)) => return Err(ReqOutcome::Proto(e)),
    };

    // QPACK-decode the header block via the shared stateful decoder. QPACK
    // failures here are connection-fatal per RFC 9204 §2.2. We advertised
    // QPACK_MAX_BLOCKED_STREAMS=0 so any block referencing inserts that
    // haven't arrived yet is a peer protocol violation.
    let decoded = {
        let decoder = state.qpack.decoder.borrow();
        decoder.decode_header(&mut block.clone()).map_err(|e| {
            ReqOutcome::Proto(ProtoError::new(
                h3_code::QPACK_DECOMPRESSION_FAILED,
                format!("qpack decode: {e:?}"),
            ))
        })?
    };

    // RFC 9204 §3.2.1: a Section Acknowledgment is required only for blocks
    // that referenced the dynamic table. For static-only blocks we skip it.
    if decoded.dyn_ref {
        let mut ack = BytesMut::with_capacity(8);
        HeaderAck(stream_id).encode(&mut ack);
        state.qpack.push_dec(&ack);
    }

    // Translate pseudo-headers + HeaderMap into an http::Request. A malformed
    // request here is a stream error, not a connection error (RFC 9114 §4.1.2).
    let header = match Header::try_from(decoded.fields) {
        Ok(h) => h,
        Err(e) => {
            reset_stream(&mut send, &mut recv, h3_code::MESSAGE_ERROR);
            return Err(ReqOutcome::Stream(Error::Proto {
                code: h3_code::MESSAGE_ERROR,
                reason: format!("malformed header: {e:?}"),
            }));
        }
    };

    let (method, uri, protocol, headers) = match header.into_request_parts() {
        Ok(parts) => parts,
        Err(e) => {
            reset_stream(&mut send, &mut recv, h3_code::MESSAGE_ERROR);
            return Err(ReqOutcome::Stream(Error::Proto {
                code: h3_code::MESSAGE_ERROR,
                reason: format!("request parts: {e:?}"),
            }));
        }
    };

    // RFC 9220: surface the :protocol pseudo-header for extended CONNECT
    // through RequestExt so user services can route on it.

    // RFC 9114 §4.1.2: validate Content-Length (single, parseable, or
    // identical duplicates). The body decoder enforces it byte-by-byte.
    let expected_len = match parse_content_length(&headers) {
        Ok(v) => v,
        Err(reason) => {
            reset_stream(&mut send, &mut recv, h3_code::MESSAGE_ERROR);
            return Err(ReqOutcome::Stream(Error::Proto {
                code: h3_code::MESSAGE_ERROR,
                reason,
            }));
        }
    };

    let body = RequestBodyV2::new(recv, leftover, expected_len);
    let mut req = Request::new(RequestExt::from_parts(
        ReqB::from(body),
        Extension::with_protocol(addr, protocol),
    ));
    *req.method_mut() = method;
    *req.uri_mut() = uri;
    *req.headers_mut() = headers;

    // Invoke user service.
    let res = service
        .call(req)
        .await
        .map_err(|e| ReqOutcome::Stream(Error::Service(e)))?;

    // Encode + stream the response back.
    if let Err(e) = send_response(&mut send, res).await {
        return Err(ReqOutcome::Stream(e));
    }

    Ok(())
}

async fn read_headers_frame(recv: &mut RecvStream) -> Result<(Bytes, BytesMut), HeadersReadError> {
    let mut buf = BytesMut::with_capacity(2048);

    loop {
        let (res, consumed) = {
            let mut cursor = Cursor::new(&buf[..]);
            let res = Frame::<PayloadLen>::decode(&mut cursor);
            (res, cursor.position() as usize)
        };

        match res {
            Ok(Frame::Headers(block)) => {
                buf.advance(consumed);
                return Ok((block, buf));
            }
            Ok(_) => {
                return Err(HeadersReadError::Connection(ProtoError::new(
                    h3_code::FRAME_UNEXPECTED,
                    "first frame on request stream was not HEADERS",
                )));
            }
            Err(FrameError::Incomplete(_)) => match recv.read_chunk(64 * 1024, true).await {
                Ok(Some(chunk)) => buf.extend_from_slice(&chunk.bytes),
                Ok(None) => {
                    return Err(HeadersReadError::StreamIncomplete(
                        "peer closed request stream before HEADERS frame".into(),
                    ));
                }
                Err(e) => {
                    return Err(HeadersReadError::StreamIncomplete(format!("request read: {e:?}")));
                }
            },
            Err(FrameError::UnknownFrame(_)) => {
                buf.advance(consumed);
            }
            Err(FrameError::UnsupportedFrame(code)) => {
                return Err(HeadersReadError::Connection(ProtoError::new(
                    h3_code::FRAME_UNEXPECTED,
                    format!("h2-only frame 0x{code:x} on request stream"),
                )));
            }
            Err(e) => {
                return Err(HeadersReadError::Connection(ProtoError::new(
                    h3_code::FRAME_ERROR,
                    format!("request frame: {e:?}"),
                )));
            }
        }
    }
}

async fn send_response<ResB, BE, S>(send: &mut SendStream, res: Response<ResB>) -> Result<(), Error<S, BE>>
where
    ResB: Body<Data = Bytes, Error = BE>,
{
    let (parts, body) = res.into_parts();

    // QPACK-encode response headers, then frame them.
    let header = Header::response(parts.status, parts.headers);
    let mut block = BytesMut::with_capacity(128);
    encode_stateless(&mut block, header).map_err(|e| Error::Proto {
        code: h3_code::INTERNAL_ERROR,
        reason: format!("qpack encode: {e:?}"),
    })?;

    let mut out = BytesMut::with_capacity(16 + block.len());
    FrameType::HEADERS.encode(&mut out);
    out.write_var(block.len() as u64);
    out.extend_from_slice(&block);
    send.write_all(&out).await.map_err(write_err)?;

    // Stream response body as DATA frames.
    let mut body = pin!(body);
    while let Some(res) = poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
        match res.map_err(Error::Body)? {
            BodyFrame::Data(bytes) => {
                let mut prefix = BytesMut::with_capacity(16);
                FrameType::DATA.encode(&mut prefix);
                prefix.write_var(bytes.len() as u64);
                send.write_all(&prefix).await.map_err(write_err)?;
                send.write_all(&bytes).await.map_err(write_err)?;
            }
            BodyFrame::Trailers(trailers) => {
                let header = Header::trailer(trailers);
                let mut block = BytesMut::with_capacity(64);
                encode_stateless(&mut block, header).map_err(|e| Error::Proto {
                    code: h3_code::INTERNAL_ERROR,
                    reason: format!("qpack encode trailer: {e:?}"),
                })?;
                let mut out = BytesMut::with_capacity(16 + block.len());
                FrameType::HEADERS.encode(&mut out);
                out.write_var(block.len() as u64);
                out.extend_from_slice(&block);
                send.write_all(&out).await.map_err(write_err)?;
            }
        }
    }

    let _ = send.finish();
    Ok(())
}

fn write_err<S, B>(e: WriteError) -> Error<S, B> {
    match e {
        WriteError::ConnectionLost(c) => Error::QuinnConnection(c),
        other => Error::Proto {
            code: h3_code::INTERNAL_ERROR,
            reason: format!("write: {other:?}"),
        },
    }
}

/// Extract the `Content-Length` value from request headers.
///
/// Per RFC 9110 §8.6 multiple values are tolerated only when identical;
/// any non-numeric value is malformed. Returns Ok(None) if the header is
/// absent.
fn parse_content_length(headers: &HeaderMap) -> Result<Option<u64>, String> {
    let mut iter = headers.get_all(header::CONTENT_LENGTH).iter();
    let Some(first) = iter.next() else {
        return Ok(None);
    };
    let s = first.to_str().map_err(|_| "non-ascii Content-Length".to_string())?;
    let n: u64 = s.parse().map_err(|_| format!("Content-Length not a u64: {s}"))?;
    for next in iter {
        if next != first {
            return Err("conflicting Content-Length values".into());
        }
    }
    Ok(Some(n))
}

fn reset_stream(send: &mut SendStream, recv: &mut RecvStream, code: u64) {
    let code = QVarInt::from_u64(code).unwrap_or(QVarInt::from_u32(0));
    let _ = send.reset(code);
    let _ = recv.stop(code);
}
