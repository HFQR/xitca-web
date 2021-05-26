use std::ops;

use tokio::io::{Interest, Ready};

/// Bit marker for tracking state of an IO type like tokio::net::TcpStream
#[derive(Copy, Clone, PartialOrd, PartialEq)]
pub(super) struct State(u8);

const READABLE: u8 = 0b0_01;
const WRITABLE: u8 = 0b0_10;
const READ_CLOSED: u8 = 0b0_0100;
const WRITE_CLOSED: u8 = 0b0_1000;

impl State {
    const READABLE: Self = Self(READABLE);
    const WRITABLE: Self = Self(WRITABLE);
    const READ_CLOSED: Self = Self(READ_CLOSED);
    const WRITE_CLOSED: Self = Self(WRITE_CLOSED);

    pub(super) fn new() -> Self {
        Self(READABLE | WRITABLE)
    }

    #[inline(always)]
    pub(super) fn interest(self) -> Interest {
        Interest::READABLE | Interest::WRITABLE
    }

    pub(super) fn set_read_close(&mut self) {
        *self |= Self::READ_CLOSED;
    }

    pub(super) fn set_write_close(&mut self) {
        *self |= Self::WRITE_CLOSED;
    }

    #[inline(always)]
    pub(super) fn readable(self) -> bool {
        self.contains(Self::READABLE)
    }

    #[inline(always)]
    pub(super) fn writeable(self) -> bool {
        self.contains(Self::WRITABLE)
    }

    pub(super) fn read_closed(self) -> bool {
        self.contains(Self::READ_CLOSED)
    }

    pub(super) fn write_closed(self) -> bool {
        self.contains(Self::WRITE_CLOSED)
    }

    #[inline(always)]
    fn contains(self, other: Self) -> bool {
        (self & other) == other
    }
}

impl ops::BitOr<State> for State {
    type Output = State;

    #[inline]
    fn bitor(self, other: State) -> State {
        State(self.0 | other.0)
    }
}

impl ops::BitAnd<State> for State {
    type Output = State;

    #[inline]
    fn bitand(self, other: State) -> State {
        State(self.0 & other.0)
    }
}

impl ops::BitOrAssign<State> for State {
    #[inline]
    fn bitor_assign(&mut self, other: State) {
        self.0 |= other.0;
    }
}
