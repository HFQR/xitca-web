use core::fmt;

use std::{error, time::Instant};

use http::{HeaderName, HeaderValue, Response, StatusCode};

use crate::{
    gcra::NotUntil,
    timer::{DefaultTimer, Timer},
};

/// Error happen when client exceeds rate limit.
#[derive(Debug)]
pub struct TooManyRequests {
    after_seconds: u64,
}

impl fmt::Display for TooManyRequests {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "too many requests. wait for {}", self.after_seconds)
    }
}

impl error::Error for TooManyRequests {}

impl From<NotUntil<Instant>> for TooManyRequests {
    fn from(e: NotUntil<Instant>) -> Self {
        let after_seconds = e.wait_time_from(DefaultTimer.now()).as_secs();

        Self { after_seconds }
    }
}

const X_RT_AFTER: HeaderName = HeaderName::from_static("x-ratelimit-after");

impl TooManyRequests {
    /// extend response headers with status code and headers
    /// StatusCode: 429
    /// Header: `x-ratelimit-after: <num in second>`
    pub fn extend_response<Ext>(&self, res: &mut Response<Ext>) {
        *res.status_mut() = StatusCode::TOO_MANY_REQUESTS;
        res.headers_mut()
            .insert(X_RT_AFTER, HeaderValue::from(self.after_seconds));
    }
}

/// Error indicating that the number of cells tested (the first
/// argument) is larger than the bucket's capacity.
///
/// This means the decision can never have a conforming result. The
/// argument gives the maximum number of cells that could ever have a
/// conforming result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsufficientCapacity(pub u32);

impl fmt::Display for InsufficientCapacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "required number of cells {} exceeds bucket's capacity",
            self.0
        )
    }
}

impl std::error::Error for InsufficientCapacity {}

#[cfg(test)]
mod test {
    use super::InsufficientCapacity;

    #[test]
    fn coverage() {
        let display_output = format!("{}", InsufficientCapacity(3));
        assert!(display_output.contains("3"));
        let debug_output = format!("{:?}", InsufficientCapacity(3));
        assert!(debug_output.contains("3"));
        assert_eq!(InsufficientCapacity(3), InsufficientCapacity(3));
    }
}
