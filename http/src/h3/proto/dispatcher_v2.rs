use core::{cell::RefCell, fmt, future::poll_fn, marker::PhantomData, net::SocketAddr, pin::pin};
use std::{io::Cursor, rc::Rc};

use quinn::{Connection, ConnectionError, RecvStream, SendStream, VarInt as QVarInt, WriteError};
use xitca_io::net::QuicStream;
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use super::{
    coding::Encode,
    frame::{Frame, FrameError, FrameType, PayloadLen, Settings},
    headers::Header,
    qpack::{decode_stateless, encode_stateless},
    stream::StreamType,
    varint::BufMutExt,
};

use crate::{
    body::{Body, Frame as BodyFrame},
    bytes::{Buf, BufMut, Bytes, BytesMut},
    error::HttpServiceError,
    h3::{body_v2::RequestBodyV2, error::Error},
    http::{Extension, Request, RequestExt, Response},
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
    pub(super) const MISSING_SETTINGS: u64 = 0x10A;
    pub(super) const REQUEST_INCOMPLETE: u64 = 0x10D;
    pub(super) const MESSAGE_ERROR: u64 = 0x10E;
    pub(super) const QPACK_DECOMPRESSION_FAILED: u64 = 0x200;
}

/// Upper bound on a single QPACK-encoded request header block we're willing to
/// decode. Prevents an unbounded allocator hit from a malicious peer.
const MAX_HEADER_BLOCK_BYTES: u64 = 64 * 1024;

/// Per-connection state shared between the dispatcher's select branches.
#[derive(Default)]
struct SharedState {
    peer_settings: Option<Settings>,
    peer_control_seen: bool,
    peer_encoder_seen: bool,
    peer_decoder_seen: bool,
    peer_goaway: Option<u64>,
}

type State = Rc<RefCell<SharedState>>;

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

        let state = Rc::new(RefCell::new(SharedState::default()));

        // Open and prime our 3 outgoing unidirectional streams. The control
        // stream also carries our initial SETTINGS frame.
        let _control = open_control_stream(&conn).await.map_err(into_err)?;
        let _encoder = open_typed_uni(&conn, StreamType::ENCODER).await.map_err(into_err)?;
        let _decoder = open_typed_uni(&conn, StreamType::DECODER).await.map_err(into_err)?;

        let mut uni_queue = Queue::new();
        let mut req_queue = Queue::new();

        let mut shutdown: Option<ProtoError> = None;

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
                    req_queue.push(handle_request_stream(send, recv, addr, service));
                }
                SelectOutput::A(SelectOutput::A(SelectOutput::A(Err(e)))) => {
                    break Err(into_err(e));
                }
                // accept_uni
                SelectOutput::A(SelectOutput::A(SelectOutput::B(Ok(recv)))) => {
                    uni_queue.push(handle_peer_uni_stream(recv, state.clone()));
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

fn into_err<S, B>(e: ConnectionError) -> Error<S, B> {
    Error::QuinnConnection(e)
}

// ---------------------------------------------------------------------------
// Unidirectional stream handling
// ---------------------------------------------------------------------------

async fn open_control_stream(conn: &Connection) -> Result<SendStream, ConnectionError> {
    let mut send = conn.open_uni().await?;

    let mut buf = BytesMut::with_capacity(StreamType::MAX_ENCODED_SIZE + Settings::MAX_ENCODED_SIZE);
    StreamType::CONTROL.encode(&mut buf);

    // initial SETTINGS — empty for now (qpack dyn table = 0, no datagrams).
    Frame::<Bytes>::Settings(Settings::default()).encode(&mut buf);

    write_all_conn(&mut send, &buf).await?;
    Ok(send)
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

async fn handle_peer_uni_stream(mut recv: RecvStream, state: State) -> Result<(), ProtoError> {
    let ty = read_stream_type(&mut recv).await?;

    match ty {
        StreamType::CONTROL => {
            mark_unique(&state, |s| &mut s.peer_control_seen, "control")?;
            run_peer_control_stream(recv, state).await
        }
        StreamType::ENCODER => {
            mark_unique(&state, |s| &mut s.peer_encoder_seen, "qpack encoder")?;
            drain_critical(recv, "qpack encoder").await
        }
        StreamType::DECODER => {
            mark_unique(&state, |s| &mut s.peer_decoder_seen, "qpack decoder")?;
            drain_critical(recv, "qpack decoder").await
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
    let mut s = state.borrow_mut();
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
            let mut state = state.borrow_mut();
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
            state.borrow_mut().peer_goaway = Some(id.into_inner());
            Ok(())
        }
        Frame::CancelPush(_) | Frame::MaxPushId(_) => Ok(()),
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

async fn drain_critical(mut recv: RecvStream, kind: &'static str) -> Result<(), ProtoError> {
    loop {
        match recv.read_chunk(64 * 1024, true).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(ProtoError::new(
                    h3_code::CLOSED_CRITICAL_STREAM,
                    format!("peer closed {kind} stream"),
                ));
            }
            Err(e) => {
                return Err(ProtoError::new(h3_code::INTERNAL_ERROR, format!("{kind} read: {e:?}")));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Bidirectional (request) stream handling
// ---------------------------------------------------------------------------

async fn handle_request_stream<'a, S, ReqB, ResB, BE>(
    mut send: SendStream,
    mut recv: RecvStream,
    addr: SocketAddr,
    service: &'a S,
) -> Result<(), ReqOutcome<S::Error, BE>>
where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    ResB: Body<Data = Bytes, Error = BE>,
    ReqB: From<RequestBodyV2>,
{
    // Read the opening HEADERS frame.
    let (block, leftover) = read_headers_frame(&mut recv).await.map_err(ReqOutcome::Proto)?;

    // QPACK-decode the header block. QPACK failures are connection-fatal
    // per RFC 9204 §2.2.
    let decoded = decode_stateless(&mut block.clone(), MAX_HEADER_BLOCK_BYTES).map_err(|e| {
        ReqOutcome::Proto(ProtoError::new(
            h3_code::QPACK_DECOMPRESSION_FAILED,
            format!("qpack decode: {e:?}"),
        ))
    })?;

    if decoded.dyn_ref {
        return Err(ReqOutcome::Proto(ProtoError::new(
            h3_code::QPACK_DECOMPRESSION_FAILED,
            "peer referenced qpack dynamic table but we advertised capacity 0",
        )));
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

    let (method, uri, _protocol, headers) = match header.into_request_parts() {
        Ok(parts) => parts,
        Err(e) => {
            reset_stream(&mut send, &mut recv, h3_code::MESSAGE_ERROR);
            return Err(ReqOutcome::Stream(Error::Proto {
                code: h3_code::MESSAGE_ERROR,
                reason: format!("request parts: {e:?}"),
            }));
        }
    };

    let body = RequestBodyV2::new(recv, leftover);
    let mut req = Request::new(RequestExt::from_parts(ReqB::from(body), Extension::new(addr)));
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

async fn read_headers_frame(recv: &mut RecvStream) -> Result<(Bytes, BytesMut), ProtoError> {
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
                return Err(ProtoError::new(
                    h3_code::FRAME_UNEXPECTED,
                    "first frame on request stream was not HEADERS",
                ));
            }
            Err(FrameError::Incomplete(_)) => match recv.read_chunk(64 * 1024, true).await {
                Ok(Some(chunk)) => buf.extend_from_slice(&chunk.bytes),
                Ok(None) => {
                    return Err(ProtoError::new(
                        h3_code::REQUEST_INCOMPLETE,
                        "peer closed request stream before HEADERS frame",
                    ));
                }
                Err(e) => {
                    return Err(ProtoError::new(h3_code::INTERNAL_ERROR, format!("request read: {e:?}")));
                }
            },
            Err(FrameError::UnknownFrame(_)) => {
                buf.advance(consumed);
            }
            Err(FrameError::UnsupportedFrame(code)) => {
                return Err(ProtoError::new(
                    h3_code::FRAME_UNEXPECTED,
                    format!("h2-only frame 0x{code:x} on request stream"),
                ));
            }
            Err(e) => {
                return Err(ProtoError::new(h3_code::FRAME_ERROR, format!("request frame: {e:?}")));
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
    encode_stateless(&mut block, header.into_iter()).map_err(|e| Error::Proto {
        code: h3_code::INTERNAL_ERROR,
        reason: format!("qpack encode: {e:?}"),
    })?;

    let mut out = BytesMut::with_capacity(16 + block.len());
    FrameType::HEADERS.encode(&mut out);
    (&mut out).write_var(block.len() as u64);
    out.extend_from_slice(&block);
    send.write_all(&out).await.map_err(write_err)?;

    // Stream response body as DATA frames.
    let mut body = pin!(body);
    while let Some(res) = poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
        match res.map_err(Error::Body)? {
            BodyFrame::Data(bytes) => {
                let mut prefix = BytesMut::with_capacity(16);
                FrameType::DATA.encode(&mut prefix);
                (&mut prefix).write_var(bytes.len() as u64);
                send.write_all(&prefix).await.map_err(write_err)?;
                send.write_all(&bytes).await.map_err(write_err)?;
            }
            BodyFrame::Trailers(trailers) => {
                let header = Header::trailer(trailers);
                let mut block = BytesMut::with_capacity(64);
                encode_stateless(&mut block, header.into_iter()).map_err(|e| Error::Proto {
                    code: h3_code::INTERNAL_ERROR,
                    reason: format!("qpack encode trailer: {e:?}"),
                })?;
                let mut out = BytesMut::with_capacity(16 + block.len());
                FrameType::HEADERS.encode(&mut out);
                (&mut out).write_var(block.len() as u64);
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

fn reset_stream(send: &mut SendStream, recv: &mut RecvStream, code: u64) {
    let code = QVarInt::from_u64(code).unwrap_or(QVarInt::from_u32(0));
    let _ = send.reset(code);
    let _ = recv.stop(code);
}
