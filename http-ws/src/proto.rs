//! Copy from [actix-http](https://github.com/actix/actix-web)

use core::fmt;

use tracing::error;

/// Operation codes as part of RFC6455.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum OpCode {
    /// Indicates a continuation frame of a fragmented message.
    Continue,
    /// Indicates a text data frame.
    Text,
    /// Indicates a binary data frame.
    Binary,
    /// Indicates a close control frame.
    Close,
    /// Indicates a ping control frame.
    Ping,
    /// Indicates a pong control frame.
    Pong,
    /// Indicates an invalid opcode was received.
    Bad,
}

impl fmt::Display for OpCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use OpCode::*;

        match self {
            Continue => write!(f, "CONTINUE"),
            Text => write!(f, "TEXT"),
            Binary => write!(f, "BINARY"),
            Close => write!(f, "CLOSE"),
            Ping => write!(f, "PING"),
            Pong => write!(f, "PONG"),
            Bad => write!(f, "BAD"),
        }
    }
}

impl From<OpCode> for u8 {
    fn from(op: OpCode) -> u8 {
        match op {
            OpCode::Continue => 0,
            OpCode::Text => 1,
            OpCode::Binary => 2,
            OpCode::Close => 8,
            OpCode::Ping => 9,
            OpCode::Pong => 10,
            OpCode::Bad => {
                error!("Attempted to convert invalid opcode to u8. This is a bug.");
                8 // if this somehow happens, a close frame will help us tear down quickly
            }
        }
    }
}

impl From<u8> for OpCode {
    fn from(byte: u8) -> OpCode {
        match byte {
            0 => OpCode::Continue,
            1 => OpCode::Text,
            2 => OpCode::Binary,
            8 => OpCode::Close,
            9 => OpCode::Ping,
            10 => OpCode::Pong,
            _ => OpCode::Bad,
        }
    }
}

/// Status code used to indicate why an endpoint is closing the WebSocket connection.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum CloseCode {
    /// Indicates a normal closure, meaning that the purpose for which the connection was
    /// established has been fulfilled.
    Normal,
    /// Indicates that an endpoint is "going away", such as a server going down or a browser having
    /// navigated away from a page.
    Away,
    /// Indicates that an endpoint is terminating the connection due to a protocol error.
    Protocol,
    /// Indicates that an endpoint is terminating the connection because it has received a type of
    /// data it cannot accept (e.g., an endpoint that understands only text data MAY send this if it
    /// receives a binary message).
    Unsupported,
    /// Indicates an abnormal closure. If the abnormal closure was due to an error, this close code
    /// will not be used. Instead, the `on_error` method of the handler will be called with
    /// the error. However, if the connection is simply dropped, without an error, this close code
    /// will be sent to the handler.
    Abnormal,
    /// Indicates that an endpoint is terminating the connection because it has received data within
    /// a message that was not consistent with the type of the message (e.g., non-UTF-8 \[RFC3629\]
    /// data within a text message).
    Invalid,
    /// Indicates that an endpoint is terminating the connection because it has received a message
    /// that violates its policy. This is a generic status code that can be returned when there is
    /// no other more suitable status code (e.g., Unsupported or Size) or if there is a need to hide
    /// specific details about the policy.
    Policy,
    /// Indicates that an endpoint is terminating the connection because it has received a message
    /// that is too big for it to process.
    Size,
    /// Indicates that an endpoint (client) is terminating the connection because it has expected
    /// the server to negotiate one or more extension, but the server didn't return them in the
    /// response message of the WebSocket handshake.  The list of extensions that are needed should
    /// be given as the reason for closing. Note that this status code is not used by the server,
    /// because it can fail the WebSocket handshake instead.
    Extension,
    /// Indicates that a server is terminating the connection because it encountered an unexpected
    /// condition that prevented it from fulfilling the request.
    Error,
    /// Indicates that the server is restarting. A client may choose to reconnect, and if it does,
    /// it should use a randomized delay of 5-30 seconds between attempts.
    Restart,
    /// Indicates that the server is overloaded and the client should either connect to a different
    /// IP (when multiple targets exist), or reconnect to the same IP when a user has performed
    /// an action.
    Again,
    #[doc(hidden)]
    Tls,
    #[doc(hidden)]
    Other(u16),
}

impl From<CloseCode> for u16 {
    fn from(code: CloseCode) -> u16 {
        match code {
            CloseCode::Normal => 1000,
            CloseCode::Away => 1001,
            CloseCode::Protocol => 1002,
            CloseCode::Unsupported => 1003,
            CloseCode::Abnormal => 1006,
            CloseCode::Invalid => 1007,
            CloseCode::Policy => 1008,
            CloseCode::Size => 1009,
            CloseCode::Extension => 1010,
            CloseCode::Error => 1011,
            CloseCode::Restart => 1012,
            CloseCode::Again => 1013,
            CloseCode::Tls => 1015,
            CloseCode::Other(code) => code,
        }
    }
}

impl From<u16> for CloseCode {
    fn from(code: u16) -> CloseCode {
        match code {
            1000 => CloseCode::Normal,
            1001 => CloseCode::Away,
            1002 => CloseCode::Protocol,
            1003 => CloseCode::Unsupported,
            1006 => CloseCode::Abnormal,
            1007 => CloseCode::Invalid,
            1008 => CloseCode::Policy,
            1009 => CloseCode::Size,
            1010 => CloseCode::Extension,
            1011 => CloseCode::Error,
            1012 => CloseCode::Restart,
            1013 => CloseCode::Again,
            1015 => CloseCode::Tls,
            _ => CloseCode::Other(code),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
/// Reason for closing the connection
pub struct CloseReason {
    /// Exit code
    pub code: CloseCode,
    /// Optional description of the exit code
    pub description: Option<String>,
}

impl From<CloseCode> for CloseReason {
    fn from(code: CloseCode) -> Self {
        CloseReason {
            code,
            description: None,
        }
    }
}

impl<T: Into<String>> From<(CloseCode, T)> for CloseReason {
    fn from(info: (CloseCode, T)) -> Self {
        CloseReason {
            code: info.0,
            description: Some(info.1.into()),
        }
    }
}

/// The WebSocket GUID as stated in the spec. See https://tools.ietf.org/html/rfc6455#section-1.3.
const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Hashes the `Sec-WebSocket-Key` header according to the WebSocket spec.
///
/// Result is a Base64 encoded byte array. `base64(sha1(input))` is always 28 bytes.
pub fn hash_key(key: &[u8]) -> [u8; 28] {
    let hash = {
        use sha1::Digest as _;

        let mut hasher = sha1::Sha1::new();

        hasher.update(key);
        hasher.update(WS_GUID);

        hasher.finalize()
    };

    let mut hash_b64 = [0; 28];
    #[allow(clippy::needless_borrow)] // clippy dumb.
    let n =
        base64::engine::Engine::encode_slice(&base64::engine::general_purpose::STANDARD, &hash, &mut hash_b64).unwrap();
    assert_eq!(n, 28);

    hash_b64
}

#[cfg(test)]
mod test {
    #![allow(unused_imports, unused_variables, dead_code)]
    use super::*;

    macro_rules! opcode_into {
        ($from:expr => $opcode:pat) => {
            match OpCode::from($from) {
                e @ $opcode => {}
                e => unreachable!("{:?}", e),
            }
        };
    }

    macro_rules! opcode_from {
        ($from:expr => $opcode:pat) => {
            let res: u8 = $from.into();
            match res {
                e @ $opcode => {}
                e => unreachable!("{:?}", e),
            }
        };
    }

    #[test]
    fn test_to_opcode() {
        opcode_into!(0 => OpCode::Continue);
        opcode_into!(1 => OpCode::Text);
        opcode_into!(2 => OpCode::Binary);
        opcode_into!(8 => OpCode::Close);
        opcode_into!(9 => OpCode::Ping);
        opcode_into!(10 => OpCode::Pong);
        opcode_into!(99 => OpCode::Bad);
    }

    #[test]
    fn test_from_opcode() {
        opcode_from!(OpCode::Continue => 0);
        opcode_from!(OpCode::Text => 1);
        opcode_from!(OpCode::Binary => 2);
        opcode_from!(OpCode::Close => 8);
        opcode_from!(OpCode::Ping => 9);
        opcode_from!(OpCode::Pong => 10);
    }

    #[test]
    #[should_panic]
    fn test_from_opcode_debug() {
        opcode_from!(OpCode::Bad => 99);
    }

    #[test]
    fn test_from_opcode_display() {
        assert_eq!(format!("{}", OpCode::Continue), "CONTINUE");
        assert_eq!(format!("{}", OpCode::Text), "TEXT");
        assert_eq!(format!("{}", OpCode::Binary), "BINARY");
        assert_eq!(format!("{}", OpCode::Close), "CLOSE");
        assert_eq!(format!("{}", OpCode::Ping), "PING");
        assert_eq!(format!("{}", OpCode::Pong), "PONG");
        assert_eq!(format!("{}", OpCode::Bad), "BAD");
    }

    #[test]
    fn test_hash_key() {
        let hash = hash_key(b"hello xitca-web");
        assert_eq!(&hash, b"z1coNb4wFSWTJ6aS4TQIOo6b9DA=");
    }

    #[test]
    fn close_code_from_u16() {
        assert_eq!(CloseCode::from(1000u16), CloseCode::Normal);
        assert_eq!(CloseCode::from(1001u16), CloseCode::Away);
        assert_eq!(CloseCode::from(1002u16), CloseCode::Protocol);
        assert_eq!(CloseCode::from(1003u16), CloseCode::Unsupported);
        assert_eq!(CloseCode::from(1006u16), CloseCode::Abnormal);
        assert_eq!(CloseCode::from(1007u16), CloseCode::Invalid);
        assert_eq!(CloseCode::from(1008u16), CloseCode::Policy);
        assert_eq!(CloseCode::from(1009u16), CloseCode::Size);
        assert_eq!(CloseCode::from(1010u16), CloseCode::Extension);
        assert_eq!(CloseCode::from(1011u16), CloseCode::Error);
        assert_eq!(CloseCode::from(1012u16), CloseCode::Restart);
        assert_eq!(CloseCode::from(1013u16), CloseCode::Again);
        assert_eq!(CloseCode::from(1015u16), CloseCode::Tls);
        assert_eq!(CloseCode::from(2000u16), CloseCode::Other(2000));
    }

    #[test]
    fn close_code_into_u16() {
        assert_eq!(1000u16, Into::<u16>::into(CloseCode::Normal));
        assert_eq!(1001u16, Into::<u16>::into(CloseCode::Away));
        assert_eq!(1002u16, Into::<u16>::into(CloseCode::Protocol));
        assert_eq!(1003u16, Into::<u16>::into(CloseCode::Unsupported));
        assert_eq!(1006u16, Into::<u16>::into(CloseCode::Abnormal));
        assert_eq!(1007u16, Into::<u16>::into(CloseCode::Invalid));
        assert_eq!(1008u16, Into::<u16>::into(CloseCode::Policy));
        assert_eq!(1009u16, Into::<u16>::into(CloseCode::Size));
        assert_eq!(1010u16, Into::<u16>::into(CloseCode::Extension));
        assert_eq!(1011u16, Into::<u16>::into(CloseCode::Error));
        assert_eq!(1012u16, Into::<u16>::into(CloseCode::Restart));
        assert_eq!(1013u16, Into::<u16>::into(CloseCode::Again));
        assert_eq!(1015u16, Into::<u16>::into(CloseCode::Tls));
        assert_eq!(2000u16, Into::<u16>::into(CloseCode::Other(2000)));
    }
}
