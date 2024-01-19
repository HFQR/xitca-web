use core::cmp;

use http::{
    header::{HeaderName, HeaderValue},
    Response,
};

use crate::{nanos::Nanos, quota::Quota};

/// Information about the rate-limiting state used to reach a decision.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RateSnapshot {
    // The "weight" of a single packet in units of time.
    t: Nanos,
    // The "burst capacity" of the bucket.
    tau: Nanos,
    // The time at which the measurement was taken.
    pub(crate) time_of_measurement: Nanos,
    // The next time a cell is expected to arrive
    pub(crate) tat: Nanos,
}

const X_RT_LIMIT: HeaderName = HeaderName::from_static("x-ratelimit-limit");
const X_RT_REMAINING: HeaderName = HeaderName::from_static("x-ratelimit-remaining");

impl RateSnapshot {
    /// extend response headers with headers
    /// Header: `x-ratelimit-limit: <num>`
    /// Header: `x-ratelimit-remaining: <num>`
    pub fn extend_response<Ext>(&self, res: &mut Response<Ext>) {
        let burst_size = self.quota().burst_size().get();
        let remaining_burst_capacity = self.remaining_burst_capacity();
        let headers = res.headers_mut();
        headers.insert(X_RT_LIMIT, HeaderValue::from(burst_size));
        headers.insert(X_RT_REMAINING, HeaderValue::from(remaining_burst_capacity));
    }

    pub(crate) const fn new(t: Nanos, tau: Nanos, time_of_measurement: Nanos, tat: Nanos) -> Self {
        Self {
            t,
            tau,
            time_of_measurement,
            tat,
        }
    }

    /// Returns the quota used to make the rate limiting decision.
    pub(crate) fn quota(&self) -> Quota {
        Quota::from_gcra_parameters(self.t, self.tau)
    }

    fn remaining_burst_capacity(&self) -> u32 {
        let t0 = self.time_of_measurement + self.t;
        (cmp::min((t0 + self.tau).saturating_sub(self.tat).as_u64(), self.tau.as_u64()) / self.t.as_u64()) as u32
    }
}

#[cfg(test)]
mod test {
    use core::time::Duration;

    use crate::{quota::Quota, state::RateLimiter, timer::FakeRelativeClock};

    #[test]
    fn state_information() {
        let clock = FakeRelativeClock::default();
        let lim = RateLimiter::direct_with_clock(Quota::per_second(4), &clock);
        assert_eq!(Ok(3), lim.check().map(|outcome| outcome.remaining_burst_capacity()));
        assert_eq!(Ok(2), lim.check().map(|outcome| outcome.remaining_burst_capacity()));
        assert_eq!(Ok(1), lim.check().map(|outcome| outcome.remaining_burst_capacity()));
        assert_eq!(Ok(0), lim.check().map(|outcome| outcome.remaining_burst_capacity()));
        assert!(lim.check().is_err());
    }

    #[test]
    fn state_snapshot_tracks_quota_accurately() {
        let period = Duration::from_millis(90);
        let quota = Quota::with_period(period).unwrap().allow_burst(2);

        let clock = FakeRelativeClock::default();

        // First test
        let lim = RateLimiter::direct_with_clock(quota, &clock);

        assert_eq!(lim.check().unwrap().remaining_burst_capacity(), 1);
        assert_eq!(lim.check().unwrap().remaining_burst_capacity(), 0);
        assert_eq!(lim.check().map_err(|_| ()), Err(()), "should rate limit");

        clock.advance(Duration::from_secs(120));
        assert_eq!(lim.check().map(|s| s.remaining_burst_capacity()), Ok(2));
        assert_eq!(lim.check().map(|s| s.remaining_burst_capacity()), Ok(1));
        assert_eq!(lim.check().map(|s| s.remaining_burst_capacity()), Ok(0));
        assert_eq!(lim.check().map_err(|_| ()), Err(()), "should rate limit");
    }

    #[test]
    fn state_snapshot_tracks_quota_accurately_with_real_clock() {
        let period = Duration::from_millis(90);
        let quota = Quota::with_period(period).unwrap().allow_burst(2);
        let lim = RateLimiter::direct(quota);

        assert_eq!(lim.check().unwrap().remaining_burst_capacity(), 1);
        assert_eq!(lim.check().unwrap().remaining_burst_capacity(), 0);
        assert_eq!(lim.check().map_err(|_| ()), Err(()), "should rate limit");
    }
}
