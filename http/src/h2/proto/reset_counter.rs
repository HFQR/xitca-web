use core::time::Duration;

use std::{collections::VecDeque, time::Instant};

/// Sliding-window counter for client-caused resets (CVE-2023-44487).
///
/// Tracks timestamps of recent resets and counts how many fall within the
/// configured window. This replaces a simple net counter (increment on
/// reset, decrement on graceful close) with time-based decay — matching
/// the approach used by the `h2` crate, Go's `net/http2`, and `nghttp2`.
///
/// A single counter is used for both peer-initiated `RST_STREAM` frames
/// and server-generated `RST_STREAM` responses to client protocol errors,
/// since both represent the same abuse signal: a client forcing the server
/// to do per-frame work.
pub(crate) struct ResetCounter {
    timestamps: VecDeque<Instant>,
    max: usize,
    window: Duration,
}

impl ResetCounter {
    pub(crate) fn new(max: usize, window: Duration) -> Self {
        Self {
            timestamps: VecDeque::new(),
            max,
            window,
        }
    }

    // driver the internal state of ResetCounter
    // return true when counter is satriated and peer has hit max resets in window
    #[cold]
    #[inline(never)]
    pub(crate) fn tick(&mut self) -> bool {
        let now = Instant::now();
        self.drain_expired(now);
        self.timestamps.push_back(now);
        self.timestamps.len() > self.max
    }

    fn drain_expired(&mut self, now: Instant) {
        while let Some(&front) = self.timestamps.front() {
            if now.saturating_duration_since(front) > self.window {
                self.timestamps.pop_front();
            } else {
                break;
            }
        }
    }
}
