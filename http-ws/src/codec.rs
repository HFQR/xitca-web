//! Copy from [actix-http](https://github.com/actix/actix-web)

use bytes::{Bytes, BytesMut};
use tracing::error;

use super::error::ProtocolError;
use super::frame::Parser;
use super::proto::{CloseReason, OpCode};

/// A WebSocket message.
#[derive(Debug, Eq, PartialEq)]
pub enum Message {
    /// Text message.
    Text(Bytes),
    /// Binary message.
    Binary(Bytes),
    /// Continuation.
    Continuation(Item),
    /// Ping message.
    Ping(Bytes),
    /// Pong message.
    Pong(Bytes),
    /// Close message with optional reason.
    Close(Option<CloseReason>),
    /// No-op. Useful for low-level services.
    Nop,
}

/// A WebSocket continuation item.
#[derive(Debug, Eq, PartialEq)]
pub enum Item {
    FirstText(Bytes),
    FirstBinary(Bytes),
    Continue(Bytes),
    Last(Bytes),
}

/// WebSocket protocol codec.
#[derive(Debug, Copy, Clone)]
pub struct Codec {
    flags: Flags,
    capacity: usize,
    max_size: usize,
}

#[derive(Debug, Copy, Clone)]
struct Flags(u8);

impl Flags {
    const SERVER: u8 = 0b0001;
    const CONTINUATION: u8 = 0b0010;

    #[inline(always)]
    fn remove(&mut self, other: u8) {
        self.0 &= !other;
    }

    #[inline(always)]
    fn insert(&mut self, other: u8) {
        self.0 |= other;
    }

    #[inline(always)]
    const fn contains(&self, other: u8) -> bool {
        (self.0 & other) == other
    }
}

impl Codec {
    /// Create new WebSocket frames decoder.
    pub const fn new() -> Codec {
        Codec {
            max_size: 65_536,
            capacity: 128,
            flags: Flags(Flags::SERVER),
        }
    }

    /// Set max frame size.
    ///
    /// By default max size is set to 64kB.
    pub fn set_max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    pub const fn max_size(&self) -> usize {
        self.max_size
    }

    /// Set capacity for concurrent buffered outgoing message.
    ///
    /// By default capacity is set to 128.
    pub fn set_capacity(mut self, size: usize) -> Self {
        self.capacity = size;
        self
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Set decoder to client mode.
    ///
    /// By default decoder works in server mode.
    pub fn client_mode(mut self) -> Self {
        self.flags.remove(Flags::SERVER);
        self.flags.remove(Flags::CONTINUATION);
        self
    }

    #[doc(hidden)]
    pub fn duplicate(mut self) -> Self {
        self.flags.remove(Flags::CONTINUATION);
        self
    }
}

impl Codec {
    pub fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), ProtocolError> {
        let mask = !self.flags.contains(Flags::SERVER);
        match item {
            Message::Text(bytes) => Parser::write_message(dst, bytes, OpCode::Text, true, mask),
            Message::Binary(bytes) => Parser::write_message(dst, bytes, OpCode::Binary, true, mask),
            Message::Ping(bytes) => Parser::write_message(dst, bytes, OpCode::Ping, true, mask),
            Message::Pong(bytes) => Parser::write_message(dst, bytes, OpCode::Pong, true, mask),
            Message::Close(reason) => Parser::write_close(dst, reason, mask),
            Message::Continuation(cont) => match cont {
                Item::Continue(_) | Item::Last(_) if !self.flags.contains(Flags::CONTINUATION) => {
                    return Err(ProtocolError::ContinuationNotStarted)
                }
                Item::FirstText(ref data) => {
                    self.try_start_continue()?;
                    Parser::write_message(dst, data, OpCode::Text, false, mask);
                }
                Item::FirstBinary(ref data) => {
                    self.try_start_continue()?;
                    Parser::write_message(dst, data, OpCode::Binary, false, mask);
                }
                Item::Continue(ref data) => Parser::write_message(dst, data, OpCode::Continue, false, mask),
                Item::Last(ref data) => {
                    self.flags.remove(Flags::CONTINUATION);
                    Parser::write_message(dst, data, OpCode::Continue, true, mask);
                }
            },
            Message::Nop => {}
        }

        Ok(())
    }

    pub fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Message>, ProtocolError> {
        match Parser::parse(src, self.flags.contains(Flags::SERVER), self.max_size)? {
            Some((finished, opcode, payload)) => match opcode {
                OpCode::Continue if !self.flags.contains(Flags::CONTINUATION) => {
                    Err(ProtocolError::ContinuationNotStarted)
                }
                OpCode::Continue => {
                    if finished {
                        self.flags.remove(Flags::CONTINUATION);
                    }
                    Ok(Some(Message::Continuation(Item::Continue(
                        payload.unwrap_or_else(Bytes::new),
                    ))))
                }
                OpCode::Binary if !finished => {
                    self.try_start_continue()?;
                    Ok(Some(Message::Continuation(Item::FirstBinary(
                        payload.unwrap_or_else(Bytes::new),
                    ))))
                }
                OpCode::Text if !finished => {
                    self.try_start_continue()?;
                    Ok(Some(Message::Continuation(Item::FirstText(
                        payload.unwrap_or_else(Bytes::new),
                    ))))
                }
                OpCode::Close if !finished => {
                    error!("Unfinished fragment {:?}", opcode);
                    Err(ProtocolError::ContinuationFragment(opcode))
                }
                OpCode::Binary => Ok(Some(Message::Binary(payload.unwrap_or_else(Bytes::new)))),
                OpCode::Text => Ok(Some(Message::Text(payload.unwrap_or_else(Bytes::new)))),
                OpCode::Close => Ok(Some(Message::Close(
                    payload.as_deref().and_then(Parser::parse_close_payload),
                ))),
                OpCode::Ping => Ok(Some(Message::Ping(payload.unwrap_or_else(Bytes::new)))),
                OpCode::Pong => Ok(Some(Message::Pong(payload.unwrap_or_else(Bytes::new)))),
                OpCode::Bad => Err(ProtocolError::BadOpCode),
            },
            None => Ok(None),
        }
    }

    fn try_start_continue(&mut self) -> Result<(), ProtocolError> {
        if !self.flags.contains(Flags::CONTINUATION) {
            self.flags.insert(Flags::CONTINUATION);
            Ok(())
        } else {
            Err(ProtocolError::ContinuationStarted)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn flag() {
        let mut flags = Flags(Flags::SERVER);

        assert!(flags.contains(Flags::SERVER));
        assert!(!flags.contains(Flags::CONTINUATION));

        flags.remove(Flags::SERVER);
        assert!(!flags.contains(Flags::SERVER));
        assert!(!flags.contains(Flags::CONTINUATION));

        flags.insert(Flags::CONTINUATION);
        assert!(flags.contains(Flags::CONTINUATION));
        assert!(!flags.contains(Flags::SERVER));
    }
}
