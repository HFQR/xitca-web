//! Copy from [actix-http](https://github.com/actix/actix-web)

use std::cell::Cell;

use bytes::{Bytes, BytesMut};
use log::error;

use super::error::ProtocolError;
use super::frame::Parser;
use super::proto::{CloseReason, OpCode};

/// A WebSocket message.
#[derive(Debug, PartialEq)]
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
#[derive(Debug, PartialEq)]
pub enum Item {
    FirstText(Bytes),
    FirstBinary(Bytes),
    Continue(Bytes),
    Last(Bytes),
}

#[derive(Debug, Clone)]
/// WebSocket protocol codec.
pub struct Codec {
    flags: Cell<Flags>,
    capacity: usize,
    max_size: usize,
}

#[derive(Debug, Copy, Clone)]
struct Flags(u8);

impl Flags {
    const SERVER: Flags = Flags(0b0000_0001);
    const CONTINUATION: Flags = Flags(0b0000_0010);
    const W_CONTINUATION: Flags = Flags(0b0000_0100);

    #[inline(always)]
    fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    #[inline(always)]
    fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[inline(always)]
    const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl Codec {
    /// Create new WebSocket frames decoder.
    pub const fn new() -> Codec {
        Codec {
            max_size: 65_536,
            capacity: 128,
            flags: Cell::new(Flags::SERVER),
        }
    }

    /// Set max frame size.
    ///
    /// By default max size is set to 64kB.
    pub fn max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    /// Set capacity for concurrent buffered outgoing message.
    ///
    /// By default capacity is set to 128.
    pub fn set_capacity(mut self, size: usize) -> Self {
        self.capacity = size;
        self
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Set decoder to client mode.
    ///
    /// By default decoder works in server mode.
    pub fn client_mode(self) -> Self {
        self.with_flags(|flags| flags.remove(Flags::SERVER));
        self
    }

    fn with_flags<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&mut Flags) -> O,
    {
        let mut flags = self.flags.get();
        let output = f(&mut flags);
        self.flags.set(flags);

        output
    }
}

impl Codec {
    pub fn encode(&self, item: Message, dst: &mut BytesMut) -> Result<(), ProtocolError> {
        match item {
            Message::Text(txt) => {
                let mask = self.with_flags(|flags| !flags.contains(Flags::SERVER));
                Parser::write_message(dst, txt, OpCode::Text, true, mask);
                Ok(())
            }
            Message::Binary(bin) => {
                let mask = self.with_flags(|flags| !flags.contains(Flags::SERVER));
                Parser::write_message(dst, bin, OpCode::Binary, true, mask);
                Ok(())
            }
            Message::Ping(txt) => {
                let mask = self.with_flags(|flags| !flags.contains(Flags::SERVER));
                Parser::write_message(dst, txt, OpCode::Ping, true, mask);
                Ok(())
            }
            Message::Pong(txt) => {
                let mask = self.with_flags(|flags| !flags.contains(Flags::SERVER));
                Parser::write_message(dst, txt, OpCode::Pong, true, mask);
                Ok(())
            }
            Message::Close(reason) => {
                let mask = self.with_flags(|flags| !flags.contains(Flags::SERVER));
                Parser::write_close(dst, reason, mask);
                Ok(())
            }
            Message::Continuation(cont) => match cont {
                Item::FirstText(data) => self.with_flags(|flags| {
                    if flags.contains(Flags::W_CONTINUATION) {
                        Err(ProtocolError::ContinuationStarted)
                    } else {
                        flags.insert(Flags::W_CONTINUATION);
                        let mask = !flags.contains(Flags::SERVER);
                        Parser::write_message(dst, &data[..], OpCode::Text, false, mask);
                        Ok(())
                    }
                }),
                Item::FirstBinary(data) => self.with_flags(|flags| {
                    if flags.contains(Flags::W_CONTINUATION) {
                        Err(ProtocolError::ContinuationStarted)
                    } else {
                        flags.insert(Flags::W_CONTINUATION);
                        let mask = !flags.contains(Flags::SERVER);
                        Parser::write_message(dst, &data[..], OpCode::Binary, false, mask);
                        Ok(())
                    }
                }),
                Item::Continue(data) => {
                    let mask = self.with_flags(|flags| {
                        if flags.contains(Flags::W_CONTINUATION) {
                            let mask = !flags.contains(Flags::SERVER);
                            Ok(mask)
                        } else {
                            Err(ProtocolError::ContinuationNotStarted)
                        }
                    })?;

                    Parser::write_message(dst, &data[..], OpCode::Continue, false, mask);
                    Ok(())
                }
                Item::Last(data) => self.with_flags(|flags| {
                    if flags.contains(Flags::W_CONTINUATION) {
                        flags.remove(Flags::W_CONTINUATION);
                        let mask = !flags.contains(Flags::SERVER);
                        Parser::write_message(dst, &data[..], OpCode::Continue, true, mask);
                        Ok(())
                    } else {
                        Err(ProtocolError::ContinuationNotStarted)
                    }
                }),
            },
            Message::Nop => Ok(()),
        }
    }

    pub fn decode(&self, src: &mut BytesMut) -> Result<Option<Message>, ProtocolError> {
        let server = self.with_flags(|flags| flags.contains(Flags::SERVER));
        match Parser::parse(src, server, self.max_size) {
            Ok(Some((finished, opcode, payload))) => {
                // continuation is not supported
                if !finished {
                    return match opcode {
                        OpCode::Continue => self.with_flags(|flags| {
                            if flags.contains(Flags::CONTINUATION) {
                                Ok(Some(Message::Continuation(Item::Continue(
                                    payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                                ))))
                            } else {
                                Err(ProtocolError::ContinuationNotStarted)
                            }
                        }),
                        OpCode::Binary => self.with_flags(|flags| {
                            if !flags.contains(Flags::CONTINUATION) {
                                flags.insert(Flags::CONTINUATION);
                                Ok(Some(Message::Continuation(Item::FirstBinary(
                                    payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                                ))))
                            } else {
                                Err(ProtocolError::ContinuationStarted)
                            }
                        }),
                        OpCode::Text => self.with_flags(|flags| {
                            if !flags.contains(Flags::CONTINUATION) {
                                flags.insert(Flags::CONTINUATION);
                                Ok(Some(Message::Continuation(Item::FirstText(
                                    payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                                ))))
                            } else {
                                Err(ProtocolError::ContinuationStarted)
                            }
                        }),
                        _ => {
                            error!("Unfinished fragment {:?}", opcode);
                            Err(ProtocolError::ContinuationFragment(opcode))
                        }
                    };
                }

                match opcode {
                    OpCode::Continue => self.with_flags(|flags| {
                        if flags.contains(Flags::CONTINUATION) {
                            flags.remove(Flags::CONTINUATION);
                            Ok(Some(Message::Continuation(Item::Last(
                                payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                            ))))
                        } else {
                            Err(ProtocolError::ContinuationNotStarted)
                        }
                    }),
                    OpCode::Bad => Err(ProtocolError::BadOpCode),
                    OpCode::Close => {
                        if let Some(ref pl) = payload {
                            let close_reason = Parser::parse_close_payload(pl);
                            Ok(Some(Message::Close(close_reason)))
                        } else {
                            Ok(Some(Message::Close(None)))
                        }
                    }
                    OpCode::Ping => Ok(Some(Message::Ping(
                        payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                    ))),
                    OpCode::Pong => Ok(Some(Message::Pong(
                        payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                    ))),
                    OpCode::Binary => Ok(Some(Message::Binary(
                        payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                    ))),
                    OpCode::Text => Ok(Some(Message::Text(
                        payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                    ))),
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn flag() {
        let mut flags = Flags::SERVER;

        assert!(flags.contains(Flags::SERVER));
        assert!(!flags.contains(Flags::CONTINUATION));
        assert!(!flags.contains(Flags::W_CONTINUATION));

        flags.remove(Flags::SERVER);
        assert!(!flags.contains(Flags::SERVER));
        assert!(!flags.contains(Flags::CONTINUATION));
        assert!(!flags.contains(Flags::W_CONTINUATION));

        flags.insert(Flags::CONTINUATION);
        assert!(flags.contains(Flags::CONTINUATION));
        assert!(!flags.contains(Flags::SERVER));
        assert!(!flags.contains(Flags::W_CONTINUATION));

        flags.insert(Flags::W_CONTINUATION);
        assert!(flags.contains(Flags::CONTINUATION));
        assert!(flags.contains(Flags::W_CONTINUATION));
        assert!(!flags.contains(Flags::SERVER));
    }
}
