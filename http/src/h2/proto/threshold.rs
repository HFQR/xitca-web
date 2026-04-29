use core::cmp::Ordering;

use super::frame::settings::{DEFAULT_INITIAL_WINDOW_SIZE, Settings};

/// Per-stream receive window threshold (75% of SETTINGS_INITIAL_WINDOW_SIZE).
/// When a stream's pending unconsumed bytes reach this level, a WINDOW_UPDATE
/// is sent. Mirrors nginx's threshold, which is widely deployed and clients
/// are tuned to work well against.
#[derive(Clone, Copy)]
pub(super) struct StreamRecvWindowThreshold(usize);

impl From<&Settings> for StreamRecvWindowThreshold {
    fn from(settings: &Settings) -> Self {
        let window = settings.initial_window_size().unwrap_or(DEFAULT_INITIAL_WINDOW_SIZE);
        let threshold = window * 3 / 4;
        Self(threshold as usize)
    }
}

impl PartialEq<StreamRecvWindowThreshold> for usize {
    fn eq(&self, other: &StreamRecvWindowThreshold) -> bool {
        self.eq(&other.0)
    }
}

impl PartialOrd<StreamRecvWindowThreshold> for usize {
    fn partial_cmp(&self, other: &StreamRecvWindowThreshold) -> Option<Ordering> {
        self.partial_cmp(&other.0)
    }
}
