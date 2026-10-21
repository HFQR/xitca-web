use core::time::Duration;

use std::{collections::VecDeque, time::Instant};

use super::frame::{reason::Reason, stream_id::StreamId};

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
///
/// It also remembers the most recent streams reset locally. The peer learns
/// of a local reset half a round trip later, so frames it sent in between
/// are unavoidable and must be discarded rather than treated as errors
/// (RFC 9113 §5.1).
pub(crate) struct ResetCounter {
    timestamps: VecDeque<Instant>,
    max: usize,
    window: Duration,
    local: LocalResets,
}

impl ResetCounter {
    pub(crate) fn new(max: usize, window: Duration, local_max: usize) -> Self {
        Self {
            timestamps: VecDeque::new(),
            max,
            window,
            local: LocalResets::new(local_max),
        }
    }

    /// Record a locally sent RST_STREAM.
    /// Returns true when the peer has caused more than max resets within the window.
    pub(crate) fn local_reset(&mut self, id: StreamId, reason: Reason) -> bool {
        self.local.push(id);

        // Not counted as they are caused by the server itself: early response (NO_ERROR)
        // and service failure (INTERNAL_ERROR).
        !matches!(reason, Reason::NO_ERROR | Reason::INTERNAL_ERROR) && self.tick()
    }

    /// Whether the stream was recently reset locally.
    pub(crate) fn is_local_reset(&self, id: &StreamId) -> bool {
        self.local.contains(id)
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

/// Fixed size ring of the most recent local resets, allocated once at
/// construction so its memory is bounded by `cap` regardless of how many
/// resets happen. The oldest entry is overwritten first.
///
/// StreamId::ZERO marks an empty slot as RST_STREAM is never sent on stream 0.
struct LocalResets {
    ids: Box<[StreamId]>,
    next: usize,
}

impl LocalResets {
    fn new(cap: usize) -> Self {
        Self {
            // at least one slot so the ring index never divides by zero.
            ids: vec![StreamId::ZERO; cap.max(1)].into_boxed_slice(),
            next: 0,
        }
    }

    fn push(&mut self, id: StreamId) {
        self.ids[self.next] = id;
        self.next = (self.next + 1) % self.ids.len();
    }

    fn contains(&self, id: &StreamId) -> bool {
        debug_assert!(!id.is_zero(), "stream 0 can not be reset");
        self.ids.contains(id)
    }
}

#[cfg(test)]
mod tests;
