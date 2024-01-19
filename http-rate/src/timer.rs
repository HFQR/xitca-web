use core::{
    fmt,
    ops::Add,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use std::{sync::Arc, time::Instant};

use super::nanos::Nanos;

/// A measurement from a clock.
pub trait Reference:
    Sized + Add<Nanos, Output = Self> + PartialEq + Eq + Ord + Copy + Clone + Send + Sync + fmt::Debug
{
    /// Determines the time that separates two measurements of a
    /// clock. Implementations of this must perform a saturating
    /// subtraction - if the `earlier` timestamp should be later,
    /// `duration_since` must return the zero duration.
    fn duration_since(&self, earlier: Self) -> Nanos;

    /// Returns a reference point that lies at most `duration` in the
    /// past from the current reference. If an underflow should occur,
    /// returns the current reference.
    fn saturating_sub(&self, duration: Nanos) -> Self;
}

/// A time source used by rate limiters.
pub trait Timer: Clone {
    /// A measurement of a monotonically increasing clock.
    type Instant: Reference;

    /// Returns a measurement of the clock.
    fn now(&self) -> Self::Instant;
}

impl Reference for Duration {
    fn duration_since(&self, earlier: Self) -> Nanos {
        self.checked_sub(earlier).unwrap_or_else(|| Duration::new(0, 0)).into()
    }

    fn saturating_sub(&self, duration: Nanos) -> Self {
        self.checked_sub(duration.into()).unwrap_or(*self)
    }
}

impl Add<Nanos> for Duration {
    type Output = Self;

    fn add(self, other: Nanos) -> Self {
        let other: Duration = other.into();
        self + other
    }
}

/// A mock implementation of a clock. All it does is keep track of
/// what "now" is (relative to some point meaningful to the program),
/// and returns that.
///
/// # Thread safety
/// The mock time is represented as an atomic u64 count of nanoseconds, behind an [`Arc`].
/// Clones of this clock will all show the same time, even if the original advances.
#[derive(Debug, Clone, Default)]
pub struct FakeRelativeClock {
    now: Arc<AtomicU64>,
}

impl FakeRelativeClock {
    #[cfg(test)]
    // Advances the fake clock by the given amount.
    pub(crate) fn advance(&self, by: Duration) {
        let by: u64 = by
            .as_nanos()
            .try_into()
            .expect("Can not represent times past ~584 years");

        let mut prev = self.now.load(Ordering::Acquire);
        let mut next = prev + by;
        while let Err(next_prev) = self
            .now
            .compare_exchange_weak(prev, next, Ordering::Release, Ordering::Relaxed)
        {
            prev = next_prev;
            next = prev + by;
        }
    }
}

impl PartialEq for FakeRelativeClock {
    fn eq(&self, other: &Self) -> bool {
        self.now.load(Ordering::Relaxed) == other.now.load(Ordering::Relaxed)
    }
}

impl Timer for FakeRelativeClock {
    type Instant = Nanos;

    fn now(&self) -> Self::Instant {
        self.now.load(Ordering::Relaxed).into()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DefaultTimer;

impl Add<Nanos> for Instant {
    type Output = Instant;

    fn add(self, other: Nanos) -> Instant {
        let other: Duration = other.into();
        self + other
    }
}

impl Reference for Instant {
    fn duration_since(&self, earlier: Self) -> Nanos {
        if earlier < *self {
            (*self - earlier).into()
        } else {
            Nanos::from(Duration::new(0, 0))
        }
    }

    fn saturating_sub(&self, duration: Nanos) -> Self {
        self.checked_sub(duration.into()).unwrap_or(*self)
    }
}

impl Timer for DefaultTimer {
    type Instant = Instant;

    fn now(&self) -> Self::Instant {
        Instant::now()
    }
}

pub trait ReasonablyRealtime: Timer {
    fn reference_point(&self) -> Self::Instant {
        self.now()
    }
}

impl ReasonablyRealtime for DefaultTimer {}

#[cfg(test)]
mod test {
    use super::*;
    use crate::nanos::Nanos;
    use std::iter::repeat;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn fake_clock_parallel_advances() {
        let clock = Arc::new(FakeRelativeClock::default());
        let threads = repeat(())
            .take(10)
            .map(move |_| {
                let clock = Arc::clone(&clock);
                thread::spawn(move || {
                    for _ in 0..1000000 {
                        let now = clock.now();
                        clock.advance(Duration::from_nanos(1));
                        assert!(clock.now() > now);
                    }
                })
            })
            .collect::<Vec<_>>();
        for t in threads {
            t.join().unwrap();
        }
    }

    #[test]
    fn duration_addition_coverage() {
        let d = Duration::from_secs(1);
        let one_ns = Nanos::new(1);
        assert!(d + one_ns > d);
    }

    #[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
    #[test]
    fn instant_impls_coverage() {
        let one_ns = Nanos::new(1);
        let c = DefaultTimer;
        let now = c.now();
        let ns_dur = Duration::from(one_ns);
        assert_ne!(now + ns_dur, now, "{:?} + {:?}", ns_dur, now);
        assert_eq!(one_ns, Reference::duration_since(&(now + one_ns), now));
        assert_eq!(Nanos::new(0), Reference::duration_since(&now, now + one_ns));
        assert_eq!(Reference::saturating_sub(&(now + Duration::from_nanos(1)), one_ns), now);
    }
}
