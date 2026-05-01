use core::{
    cmp::Ordering,
    ops::{Add, AddAssign, Sub, SubAssign},
};

use super::frame::settings::{DEFAULT_INITIAL_WINDOW_SIZE, MAX_INITIAL_WINDOW_SIZE};

/// HTTP/2 receive flow-control window (RFC 7540 §6.9). 31-bit unsigned.
/// Underflow on `try_dec` is a flow-control violation.
#[derive(Clone, Copy)]
pub(crate) struct RecvWindow(u32);

impl RecvWindow {
    pub(crate) const ZERO: Self = Self::new(0);

    pub(super) const fn new(v: u32) -> Self {
        Self(v)
    }

    pub(super) const fn value(self) -> u32 {
        self.0
    }

    /// Saturating subtraction. Returns `ZERO` when `rhs > self`.
    pub(super) fn saturating_sub(self, rhs: RecvWindow) -> RecvWindow {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub(super) fn try_dec(&mut self, n: RecvWindow) -> Result<(), ()> {
        self.0 = self.0.checked_sub(n.0).ok_or(())?;
        Ok(())
    }
}

impl Default for RecvWindow {
    fn default() -> Self {
        Self(DEFAULT_INITIAL_WINDOW_SIZE)
    }
}

impl Add for RecvWindow {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for RecvWindow {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0
    }
}

impl AddAssign<u32> for RecvWindow {
    #[inline]
    fn add_assign(&mut self, rhs: u32) {
        self.0 += rhs;
    }
}

impl Sub for RecvWindow {
    type Output = Self;

    fn sub(mut self, rhs: Self) -> Self::Output {
        self.0 -= rhs.0;
        self
    }
}

impl PartialEq for RecvWindow {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl PartialOrd for RecvWindow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

/// HTTP/2 send flow-control window (RFC 7540 §6.9.2). 31-bit signed —
/// can become negative when the peer shrinks SETTINGS_INITIAL_WINDOW_SIZE
/// with bytes in flight.
#[derive(Clone, Copy)]
pub(super) struct SendWindow(i32);

impl SendWindow {
    pub(super) const ZERO: Self = Self::new(0);

    pub(super) const fn new(v: i32) -> Self {
        Self(v)
    }

    /// Construct from an unsigned wire value. Panics if `v` exceeds
    /// `MAX_INITIAL_WINDOW_SIZE` (RFC 7540 §6.9.1, 2^31-1) — the upper bound
    /// the wire format permits for any SendWindow-typed value (flow-control
    /// window or SETTINGS_MAX_FRAME_SIZE, both ≤ this limit).
    pub(super) const fn from_u32(v: u32) -> Self {
        assert!(
            v <= MAX_INITIAL_WINDOW_SIZE as u32,
            "SendWindow exceeds RFC 7540 §6.9.1 maximum (2^31-1)"
        );
        Self::new(v as i32)
    }

    /// Construct from a `usize` byte count, saturating at
    /// `MAX_INITIAL_WINDOW_SIZE`. Use for values that may legitimately
    /// exceed the protocol limit (e.g. an arbitrarily large `Bytes::len()`);
    /// the caller paces the actual transfer across multiple iterations.
    pub(super) const fn from_usize_saturating(v: usize) -> Self {
        let clamped = if v > MAX_INITIAL_WINDOW_SIZE {
            MAX_INITIAL_WINDOW_SIZE
        } else {
            v
        };
        Self::new(clamped as i32)
    }

    pub(super) fn is_positive(self) -> bool {
        self > Self::ZERO
    }

    /// Apply a peer WINDOW_UPDATE. Errors if the result would exceed
    /// MAX_INITIAL_WINDOW_SIZE (RFC §6.9.1, FLOW_CONTROL_ERROR).
    pub(super) fn try_inc(&mut self, incr: SendWindow) -> Result<(), ()> {
        let next = self.0 as i64 + incr.0 as i64;
        if next > MAX_INITIAL_WINDOW_SIZE as i64 {
            return Err(());
        }
        self.0 = next as i32;
        Ok(())
    }

    /// Apply a SETTINGS_INITIAL_WINDOW_SIZE delta to a live stream send
    /// window. May drive the window negative.
    pub(super) fn apply_initial_delta(&mut self, delta: SendWindow) {
        self.0 = self.0.saturating_add(delta.0);
    }

    pub(super) fn as_frame_size(self) -> usize {
        self.0 as usize
    }
}

impl Default for SendWindow {
    fn default() -> Self {
        Self::from_u32(DEFAULT_INITIAL_WINDOW_SIZE)
    }
}

impl Eq for SendWindow {}

impl Ord for SendWindow {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl Sub for SendWindow {
    type Output = Self;

    #[inline]
    fn sub(mut self, rhs: Self) -> Self::Output {
        self.0 -= rhs.0;
        self
    }
}

impl SubAssign for SendWindow {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 = self.0 - rhs.0;
    }
}

impl PartialEq for SendWindow {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl PartialOrd for SendWindow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
