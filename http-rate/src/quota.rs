use core::{convert::TryInto, fmt, num::NonZeroU32, time::Duration};

use crate::nanos::Nanos;

/// A rate-limiting quota.
///
/// Quotas are expressed in a positive number of "cells" (the maximum number of positive decisions /
/// allowed items until the rate limiter needs to replenish) and the amount of time for the rate
/// limiter to replenish a single cell.
///
/// Neither the number of cells nor the replenishment unit of time may be zero.
///
/// # Burst sizes
/// There are multiple ways of expressing the same quota: a quota given as `Quota::per_second(1)`
/// allows, on average, the same number of cells through as a quota given as `Quota::per_minute(60)`.
/// However, the quota of `Quota::per_minute(60)` has a burst size of 60 cells, meaning it is
/// possible to accommodate 60 cells in one go, after which the equivalent of a minute of inactivity
/// is required for the burst allowance to be fully restored.
///
/// Burst size gets really important when you construct a rate limiter that should allow multiple
/// elements through at one time (using [`RateLimiter.check_n`](struct.RateLimiter.html#method.check_n)
/// and its related functions): Only
/// at most as many cells can be let through in one call as are given as the burst size.
///
/// In other words, the burst size is the maximum number of cells that the rate limiter will ever
/// allow through without replenishing them.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Quota {
    pub(crate) max_burst: NonZeroU32,
    pub(crate) replenish_1_per: Duration,
}

impl Quota {
    /// Construct a quota for a number of cells per second. The given number of cells is also
    /// assumed to be the maximum burst size.
    ///
    /// # Panics
    /// - When max_burst is zero.
    pub fn per_second<B>(max_burst: B) -> Self
    where
        B: TryInto<NonZeroU32>,
        B::Error: fmt::Debug,
    {
        let max_burst = max_burst.try_into().unwrap();
        let replenish_interval_ns = Duration::from_secs(1).as_nanos() / (max_burst.get() as u128);
        Self {
            max_burst,
            replenish_1_per: Duration::from_nanos(replenish_interval_ns as u64),
        }
    }

    /// Construct a quota for a number of cells per 60-second period. The given number of cells is
    /// also assumed to be the maximum burst size.
    ///
    /// # Panics
    /// - When max_burst is zero.
    pub fn per_minute<B>(max_burst: B) -> Self
    where
        B: TryInto<NonZeroU32>,
        B::Error: fmt::Debug,
    {
        let max_burst = max_burst.try_into().unwrap();
        let replenish_interval_ns = Duration::from_secs(60).as_nanos() / (max_burst.get() as u128);
        Quota {
            max_burst,
            replenish_1_per: Duration::from_nanos(replenish_interval_ns as u64),
        }
    }

    /// Construct a quota for a number of cells per 60-minute (3600-second) period. The given number
    /// of cells is also assumed to be the maximum burst size.
    ///
    /// # Panics
    /// - When max_burst is zero.
    pub fn per_hour<B>(max_burst: B) -> Self
    where
        B: TryInto<NonZeroU32>,
        B::Error: fmt::Debug,
    {
        let max_burst = max_burst.try_into().unwrap();
        let replenish_interval_ns = Duration::from_secs(60 * 60).as_nanos() / (max_burst.get() as u128);
        Self {
            max_burst,
            replenish_1_per: Duration::from_nanos(replenish_interval_ns as u64),
        }
    }

    /// Construct a quota that replenishes one cell in a given
    /// interval.
    ///
    /// This constructor is meant to replace [`::new`](#method.new),
    /// in cases where a longer refresh period than 1 cell/hour is
    /// necessary.
    ///
    /// If the time interval is zero, returns `None`.
    pub fn with_period(replenish_1_per: Duration) -> Option<Quota> {
        if replenish_1_per.as_nanos() == 0 {
            None
        } else {
            Some(Quota {
                max_burst: NonZeroU32::new(1).unwrap(),
                replenish_1_per,
            })
        }
    }

    /// Adjusts the maximum burst size for a quota to construct a rate limiter with a capacity
    /// for at most the given number of cells.
    ///
    /// # Panics
    /// - When max_burst is zero.
    pub fn allow_burst<B>(self, max_burst: B) -> Self
    where
        B: TryInto<NonZeroU32>,
        B::Error: fmt::Debug,
    {
        let max_burst = max_burst.try_into().unwrap();
        Self { max_burst, ..self }
    }
}

impl Quota {
    // The maximum number of cells that can be allowed in one burst.
    pub(crate) const fn burst_size(&self) -> NonZeroU32 {
        self.max_burst
    }

    #[cfg(test)]
    // The time it takes for a rate limiter with an exhausted burst budget to replenish
    // a single element.
    const fn replenish_interval(&self) -> Duration {
        self.replenish_1_per
    }

    // The time it takes to replenish the entire maximum burst size.
    // const fn burst_size_replenished_in(&self) -> Duration {
    //     let fill_in_ns = self.replenish_1_per.as_nanos() * self.max_burst.get() as u128;
    //     Duration::from_nanos(fill_in_ns as u64)
    // }
}

impl Quota {
    // A way to reconstruct a Quota from an in-use Gcra.
    pub(crate) fn from_gcra_parameters(t: Nanos, tau: Nanos) -> Quota {
        let max_burst = NonZeroU32::new((tau.as_u64() / t.as_u64()) as u32).unwrap();
        let replenish_1_per = t.into();
        Quota {
            max_burst,
            replenish_1_per,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn time_multiples() {
        let hourly = Quota::per_hour(1);
        let minutely = Quota::per_minute(1);
        let secondly = Quota::per_second(1);

        assert_eq!(hourly.replenish_interval() / 60, minutely.replenish_interval());
        assert_eq!(minutely.replenish_interval() / 60, secondly.replenish_interval());
    }

    #[test]
    fn period_error_cases() {
        assert!(Quota::with_period(Duration::from_secs(0)).is_none());
    }
}
