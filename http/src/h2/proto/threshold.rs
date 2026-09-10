use core::cmp::Ordering;

use super::window::RecvWindow;

/// Batch consumed stream credit until it reaches approximately 75% of the initial stream window.
/// Connection credit is batched separately by the writer without a threshold.
#[derive(Clone, Copy)]
pub(super) struct RecvWindowThreshold(RecvWindow);

impl From<RecvWindow> for RecvWindowThreshold {
    fn from(window: RecvWindow) -> Self {
        let threshold = window.value() / 4 * 3;
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
