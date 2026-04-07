use core::fmt;

use std::error;

use super::proto::OpCode;

/// WebSocket protocol errors.
///
/// # Close handshake errors
///
/// [`SendClosed`](Self::SendClosed), [`RecvClosed`](Self::RecvClosed), and
/// [`UnexpectedEof`](Self::UnexpectedEof) are related to the WebSocket close handshake.
/// Their meaning depends on the context in which they are observed:
///
/// ## `SendClosed`
///
/// Returned by [`ResponseSender::send`] when a Close frame has already been sent on this
/// session. This can happen in two scenarios:
///
/// - **You called `close()` and then tried to send more messages.** After sending a Close
///   frame, no further messages are permitted per RFC 6455.
/// - **A concurrent sender (via [`ResponseWeakSender::upgrade`]) already initiated the
///   close.** The caller observing this error should return gracefully — the shutdown is
///   already in progress.
///
/// ## `RecvClosed`
///
/// Returned by [`RequestStream`] (as `WsError::Protocol(ProtocolError::RecvClosed)`) when
/// the stream is polled after a Close frame has already been received and yielded. The
/// caller should have stopped polling after observing `Message::Close`. Receiving this
/// error means:
///
/// - The remote peer sent a Close frame, which was already yielded as
///   `Ok(Message::Close(_))`.
/// - The caller should respond with a Close frame via [`ResponseSender`] (if not already
///   sent) and then drop both the stream and sender.
///
/// ## `UnexpectedEof`
///
/// Returned by [`RequestStream`] when the underlying transport closed without a Close
/// frame. This is an abnormal closure — the remote peer disconnected without following the
/// WebSocket protocol. The associated connection should not be reused.
///
/// # Typical close flows
///
/// **Remote-initiated close:**
/// 1. [`RequestStream`] yields `Ok(Message::Close(reason))`.
/// 2. Caller sends a Close frame back via [`ResponseSender::close`].
/// 3. Caller drops both the stream and sender.
///
/// **Local-initiated close:**
/// 1. Caller calls [`ResponseSender::close`].
/// 2. Caller continues polling [`RequestStream`] for the peer's Close echo.
/// 3. [`RequestStream`] yields `Ok(Message::Close(_))` — handshake complete.
/// 4. Caller drops the stream. Any concurrent senders attempting to send will observe
///    `SendClosed`.
///
/// **Timeout on close echo:**
/// 1. Caller calls [`ResponseSender::close`].
/// 2. Caller polls [`RequestStream`] with a timeout.
/// 3. Timeout expires — caller sends an error via [`ResponseSender::send_error`] to signal
///    the I/O layer to shut down the connection.
#[derive(Debug)]
pub enum ProtocolError {
    UnmaskedFrame,
    MaskedFrame,
    InvalidOpcode(u8),
    InvalidLength(usize),
    BadOpCode,
    Overflow,
    ContinuationNotStarted,
    ContinuationStarted,
    ContinuationFragment(OpCode),
    /// A Close frame has already been sent. No further messages can be sent.
    SendClosed,
    /// A Close frame has already been received. The caller should stop polling the stream.
    RecvClosed,
    /// The underlying transport closed without a Close frame. This is an abnormal closure.
    UnexpectedEof,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnmaskedFrame => f.write_str("Received an unmasked frame from client."),
            Self::MaskedFrame => f.write_str("Received a masked frame from server."),
            Self::InvalidOpcode(code) => write!(f, " Encountered invalid OpCode: {code}"),
            Self::InvalidLength(len) => write!(f, "Invalid control frame length: {len}."),
            Self::BadOpCode => f.write_str("Bad opcode."),
            Self::Overflow => f.write_str("A payload reached size limit."),
            Self::ContinuationNotStarted => f.write_str("Continuation is not started."),
            Self::ContinuationStarted => f.write_str("Received new continuation but it is already started."),
            Self::ContinuationFragment(ref code) => write!(f, "Unknown continuation fragment with OpCode: {code}."),
            Self::SendClosed => f.write_str("Close message has already been sent."),
            Self::RecvClosed => f.write_str("Close message has alredy been received"),
            Self::UnexpectedEof => f.write_str("Connection is closed prematurely without Close message"),
        }
    }
}

impl error::Error for ProtocolError {}

impl From<OpCode> for ProtocolError {
    fn from(e: OpCode) -> Self {
        Self::ContinuationFragment(e)
    }
}

/// WebSocket handshake errors
#[derive(Debug, Eq, PartialEq)]
pub enum HandshakeError {
    GetMethodRequired,
    ConnectMethodRequired,
    NoWebsocketUpgrade,
    NoConnectionUpgrade,
    NoVersionHeader,
    UnsupportedVersion,
    BadWebsocketKey,
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::GetMethodRequired => f.write_str("Only GET method is allowed for HTTP/1.1 websocket."),
            Self::ConnectMethodRequired => f.write_str("Only CONNECT method is allowed HTTP/2 websocket."),
            Self::NoWebsocketUpgrade => f.write_str("Upgrade header is not set to HTTP/1.1 websocket."),
            Self::NoConnectionUpgrade => f.write_str("Connection header is not set to HTTP/1.1 websocket."),
            Self::NoVersionHeader => f.write_str(" WebSocket version header is not set to HTTP/1.1 websocket."),
            Self::UnsupportedVersion => f.write_str("Unsupported WebSocket version."),
            Self::BadWebsocketKey => f.write_str("WebSocket key is not set or wrong to HTTP/1.1 websocket."),
        }
    }
}

impl error::Error for HandshakeError {}
