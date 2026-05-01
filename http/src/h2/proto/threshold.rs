use core::cmp::Ordering;

use super::{
    frame::settings::{DEFAULT_INITIAL_WINDOW_SIZE, Settings},
    window::RecvWindow,
};

/// Per-stream receive window threshold (75% of SETTINGS_INITIAL_WINDOW_SIZE).
/// When a stream's pending unconsumed bytes reach this level, a WINDOW_UPDATE
/// is sent. Mirrors nginx's threshold, which is widely deployed and clients
/// are tuned to work well against.
#[derive(Clone, Copy)]
pub(super) struct RecvWindowThreshold(RecvWindow);

impl From<&Settings> for RecvWindowThreshold {
    fn from(settings: &Settings) -> Self {
        let window = settings.initial_window_size().unwrap_or(DEFAULT_INITIAL_WINDOW_SIZE);
        let threshold = window * 3 / 4;
        Self(RecvWindow::new(threshold))
    }
}

impl PartialEq<RecvWindowThreshold> for RecvWindow {
    fn eq(&self, other: &RecvWindowThreshold) -> bool {
        self.eq(&other.0)
    }
}

impl PartialOrd<RecvWindowThreshold> for RecvWindow {
    fn partial_cmp(&self, other: &RecvWindowThreshold) -> Option<Ordering> {
        self.partial_cmp(&other.0)
    }
}
