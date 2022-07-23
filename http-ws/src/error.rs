use std::{error, fmt, io};

use super::proto::OpCode;

/// WebSocket protocol errors.
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
    Io(io::Error),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnmaskedFrame => write!(f, "Received an unmasked frame from client."),
            Self::MaskedFrame => write!(f, "Received a masked frame from server."),
            Self::InvalidOpcode(code) => write!(f, " Encountered invalid OpCode: {}", code),
            Self::InvalidLength(len) => write!(f, "Invalid control frame length: {}.", len),
            Self::BadOpCode => write!(f, "Bad opcode."),
            Self::Overflow => write!(f, "A payload reached size limit."),
            Self::ContinuationNotStarted => write!(f, "Continuation is not started."),
            Self::ContinuationStarted => write!(f, "Received new continuation but it is already started."),
            Self::ContinuationFragment(ref code) => write!(f, "Unknown continuation fragment with OpCode: {}.", code),
            Self::Io(ref e) => write!(f, "Io error: {}", e),
        }
    }
}

impl error::Error for ProtocolError {}

impl From<OpCode> for ProtocolError {
    fn from(e: OpCode) -> Self {
        Self::ContinuationFragment(e)
    }
}

impl From<io::Error> for ProtocolError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
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
            Self::GetMethodRequired => write!(f, "Only GET method is allowed for HTTP/1.1 websocket."),
            Self::ConnectMethodRequired => write!(f, "Only CONNECT method is allowed HTTP/2 websocket."),
            Self::NoWebsocketUpgrade => write!(f, "Upgrade header is not set to HTTP/1.1 websocket."),
            Self::NoConnectionUpgrade => write!(f, "Connection header is not set to HTTP/1.1 websocket."),
            Self::NoVersionHeader => write!(f, " WebSocket version header is not set to HTTP/1.1 websocket."),
            Self::UnsupportedVersion => write!(f, "Unsupported WebSocket version."),
            Self::BadWebsocketKey => write!(f, "WebSocket key is not set or wrong to HTTP/1.1 websocket."),
        }
    }
}

impl error::Error for HandshakeError {}
