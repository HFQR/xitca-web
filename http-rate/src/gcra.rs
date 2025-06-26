use core::{cmp, fmt, time::Duration};

use crate::{nanos::Nanos, quota::Quota, snapshot::RateSnapshot, state::StateStore, timer::Reference};

#[cfg(test)]
use core::num::NonZeroU32;

#[cfg(test)]
use crate::error::InsufficientCapacity;

/// A negative rate-limiting outcome.
///
/// `NotUntil`'s methods indicate when a caller can expect the next positive
/// rate-limiting result.
#[derive(Debug, PartialEq, Eq)]
pub struct NotUntil<P: Reference> {
    state: RateSnapshot,
    start: P,
}

impl<P> NotUntil<P>
where
    P: Reference,
{
    /// Create a `NotUntil` as a negative rate-limiting result.
    #[inline]
    pub(crate) fn new(state: RateSnapshot, start: P) -> Self {
        Self { state, start }
    }

    /// Returns the earliest time at which a decision could be
    /// conforming (excluding conforming decisions made by the Decider
    /// that are made in the meantime).
    #[inline]
    pub fn earliest_possible(&self) -> P {
        let tat: Nanos = self.state.tat;
        self.start + tat
    }

    /// Returns the minimum amount of time from the time that the
    /// decision was made that must pass before a
    /// decision can be conforming.
    ///
    /// If the time of the next expected positive result is in the past,
    /// `wait_time_from` returns a zero `Duration`.
    #[inline]
    pub fn wait_time_from(&self, from: P) -> Duration {
        let earliest = self.earliest_possible();
        earliest.duration_since(earliest.min(from)).into()
    }

    /// Returns the rate limiting [`Quota`] used to reach the decision.
    #[inline]
    pub fn quota(&self) -> Quota {
        self.state.quota()
    }
}

impl<P> fmt::Display for NotUntil<P>
where
    P: Reference,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "rate-limited until {:?}", self.start + self.state.tat)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Gcra {
    /// The "weight" of a single packet in units of time.
    t: Nanos,

    /// The "burst capacity" of the bucket.
    tau: Nanos,
}

impl Gcra {
    pub(crate) fn new(quota: Quota) -> Self {
        let tau: Nanos = (cmp::max(quota.replenish_1_per, Duration::from_nanos(1)) * quota.max_burst.get()).into();
        let t: Nanos = quota.replenish_1_per.into();
        Gcra { t, tau }
    }

    /// Computes and returns a new ratelimiter state if none exists yet.
    fn starting_state(&self, t0: Nanos) -> Nanos {
        t0 + self.t
    }

    /// Tests a single cell against the rate limiter state and updates it at the given key.
    pub(crate) fn test_and_update<K, P, S>(
        &self,
        start: P,
        key: &K,
        state: &S,
        t0: P,
    ) -> Result<RateSnapshot, NotUntil<P>>
    where
        P: Reference,
        S: StateStore<Key = K>,
    {
        let t0 = t0.duration_since(start);
        let tau = self.tau;
        let t = self.t;
        state.measure_and_replace(key, |tat| {
            let tat = tat.unwrap_or_else(|| self.starting_state(t0));
            let earliest_time = tat.saturating_sub(tau);
            if t0 < earliest_time {
                let state = RateSnapshot::new(self.t, self.tau, earliest_time, earliest_time);
                Err(NotUntil::new(state, start))
            } else {
                let next = cmp::max(tat, t0) + t;
                Ok((RateSnapshot::new(self.t, self.tau, t0, next), next))
            }
        })
    }

    #[cfg(test)]
    /// Tests whether all `n` cells could be accommodated and updates the rate limiter state, if so.
    pub(crate) fn test_n_all_and_update<K, P, S>(
        &self,
        start: P,
        key: &K,
        n: NonZeroU32,
        state: &S,
        t0: P,
    ) -> Result<Result<RateSnapshot, NotUntil<P>>, InsufficientCapacity>
    where
        P: Reference,
        S: StateStore<Key = K>,
    {
        let t0 = t0.duration_since(start);
        let tau = self.tau;
        let t = self.t;
        let additional_weight = t * (n.get() - 1) as u64;

        // check that we can allow enough cells through. Note that `additional_weight` is the
        // value of the cells *in addition* to the first cell - so add that first cell back.
        if additional_weight + t > tau {
            return Err(InsufficientCapacity((tau.as_u64() / t.as_u64()) as u32));
        }
        Ok(state.measure_and_replace(key, |tat| {
            let tat = tat.unwrap_or_else(|| self.starting_state(t0));
            let earliest_time = (tat + additional_weight).saturating_sub(tau);
            if t0 < earliest_time {
                let state = RateSnapshot::new(self.t, self.tau, earliest_time, earliest_time);
                Err(NotUntil::new(state, start))
            } else {
                let next = cmp::max(tat, t0) + t + additional_weight;
                Ok((RateSnapshot::new(self.t, self.tau, t0, next), next))
            }
        }))
    }
}

#[cfg(test)]
mod test {
    use proptest::prelude::*;

    use super::*;

    /// Exercise derives and convenience impls on Gcra to make coverage happy
    #[test]
    fn gcra_derives() {
        use all_asserts::assert_gt;

        let g = Gcra::new(Quota::per_second(1));
        let g2 = Gcra::new(Quota::per_second(2));
        assert_eq!(g, g);
        assert_ne!(g, g2);
        assert_gt!(format!("{:?}", g).len(), 0);
    }

    /// Exercise derives and convenience impls on NotUntil to make coverage happy
    #[test]
    fn notuntil_impls() {
        use all_asserts::assert_gt;

        use crate::{state::RateLimiter, timer::FakeRelativeClock};

        let clock = FakeRelativeClock::default();
        let quota = Quota::per_second(1);
        let lb = RateLimiter::direct_with_clock(quota, &clock);
        assert!(lb.check().is_ok());
        assert!(lb
            .check()
            .map_err(|nu| {
                assert_eq!(nu, nu);
                assert_gt!(format!("{nu:?}").len(), 0);
                assert_eq!(format!("{nu}"), "rate-limited until Nanos(1s)");
                assert_eq!(nu.quota(), quota);
            })
            .is_err());
    }

    #[derive(Debug)]
    struct Count(NonZeroU32);
    impl Arbitrary for Count {
        type Parameters = ();
        fn arbitrary_with(_args: ()) -> Self::Strategy {
            (1..10000u32).prop_map(|x| Count(NonZeroU32::new(x).unwrap())).boxed()
        }

        type Strategy = BoxedStrategy<Count>;
    }

    #[test]
    fn cover_count_derives() {
        assert_eq!(format!("{:?}", Count(NonZeroU32::new(1).unwrap())), "Count(1)");
    }

    #[test]
    fn roundtrips_quota() {
        proptest!(ProptestConfig::default(), |(per_second: Count, burst: Count)| {
            let quota = Quota::per_second(per_second.0).allow_burst(burst.0);
            let gcra = Gcra::new(quota);
            let back = Quota::from_gcra_parameters(gcra.t, gcra.tau);
            assert_eq!(quota, back);
        })
    }
}
