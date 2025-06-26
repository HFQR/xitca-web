#![doc=include_str!( "../README.md")]
#![allow(clippy::declare_interior_mutable_const)]

mod error;
mod gcra;
mod nanos;
mod quota;
mod snapshot;
mod state;
mod timer;

pub use error::TooManyRequests;
pub use quota::Quota;
pub use snapshot::RateSnapshot;

use core::net::{IpAddr, SocketAddr};

use std::sync::Arc;

use http::header::{HeaderMap, HeaderName, FORWARDED};

use crate::state::{keyed::DefaultKeyedStateStore, RateLimiter};

#[derive(Clone)]
pub struct RateLimit {
    limit: Arc<RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>>>,
}

impl RateLimit {
    /// Construct a new RateLimit with given quota.
    pub fn new(quota: Quota) -> Self {
        Self {
            limit: Arc::new(RateLimiter::hashmap(quota)),
        }
    }

    /// Rate limit [Request] based on it's [HeaderMap] state and given client [SocketAddr]
    /// "x-real-ip", "x-forwarded-for" and "forwarded" headers are checked in order start
    /// from left to determine client's socket address. Received [SocketAddr] will be used
    /// as fallback when all headers are absent or can't provide valid client address.
    ///
    /// [Request]: http::Request
    pub fn rate_limit(&self, headers: &HeaderMap, addr: &SocketAddr) -> Result<RateSnapshot, TooManyRequests> {
        let addr = maybe_x_forwarded_for(headers)
            .or_else(|| maybe_x_real_ip(headers))
            .or_else(|| maybe_forwarded(headers))
            .unwrap_or_else(|| addr.ip());
        self.limit.check_key(&addr).map_err(TooManyRequests::from)
    }
}

const X_REAL_IP: HeaderName = HeaderName::from_static("x-real-ip");
const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

fn maybe_x_forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_FORWARDED_FOR)
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.split(',').find_map(|s| s.trim().parse().ok()))
}

fn maybe_x_real_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_REAL_IP)
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.parse().ok())
}

fn maybe_forwarded(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all(FORWARDED)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|val| val.split(';'))
        .flat_map(|p| p.split(','))
        .map(|val| val.trim().splitn(2, '='))
        .find_map(|mut val| match (val.next(), val.next()) {
            (Some(name), Some(val)) if name.trim().eq_ignore_ascii_case("for") => {
                let val = val.trim();
                val.parse::<IpAddr>()
                    .or_else(|_| val.parse::<SocketAddr>().map(|addr| addr.ip()))
                    .ok()
            }
            _ => None,
        })
}

#[cfg(test)]
type DefaultDirectRateLimiter = RateLimiter<state::direct::NotKeyed, state::InMemoryState>;

#[cfg(test)]
mod test {
    use core::{num::NonZeroU32, time::Duration};

    use std::thread;

    use all_asserts::*;
    use http::header::HeaderValue;

    use crate::{
        error::InsufficientCapacity,
        timer::{DefaultTimer, FakeRelativeClock, Timer},
    };

    use super::*;

    #[test]
    fn forwarded_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            FORWARDED,
            HeaderValue::from_static("for =192.0.2.60;proto=http;by=203.0.113.43"),
        );
        assert_eq!(maybe_forwarded(&headers).unwrap().to_string(), "192.0.2.60");
    }

    #[test]
    fn rejects_too_many() {
        let clock = FakeRelativeClock::default();
        let lb = RateLimiter::direct_with_clock(Quota::per_second(2), &clock);
        let ms = Duration::from_millis(1);

        // use up our burst capacity (2 in the first second):
        assert!(lb.check().is_ok(), "Now: {:?}", clock.now());
        clock.advance(ms);
        assert!(lb.check().is_ok(), "Now: {:?}", clock.now());

        clock.advance(ms);
        assert!(lb.check().is_err(), "Now: {:?}", clock.now());

        // should be ok again in 1s:
        clock.advance(ms * 1000);
        assert!(lb.check().is_ok(), "Now: {:?}", clock.now());
        clock.advance(ms);
        assert!(lb.check().is_ok());

        clock.advance(ms);
        assert!(lb.check().is_err(), "{lb:?}");
    }

    #[test]
    fn all_1_identical_to_1() {
        let clock = FakeRelativeClock::default();
        let lb = RateLimiter::direct_with_clock(Quota::per_second(2), &clock);
        let ms = Duration::from_millis(1);
        let one = NonZeroU32::new(1).unwrap();

        // use up our burst capacity (2 in the first second):
        assert!(lb.check_n(one).unwrap().is_ok(), "Now: {:?}", clock.now());
        clock.advance(ms);
        assert!(lb.check_n(one).unwrap().is_ok(), "Now: {:?}", clock.now());

        clock.advance(ms);
        assert!(lb.check_n(one).unwrap().is_err(), "Now: {:?}", clock.now());

        // should be ok again in 1s:
        clock.advance(ms * 1000);
        assert!(lb.check_n(one).unwrap().is_ok(), "Now: {:?}", clock.now());
        clock.advance(ms);
        assert!(lb.check_n(one).unwrap().is_ok());

        clock.advance(ms);
        assert!(lb.check_n(one).unwrap().is_err(), "{lb:?}");
    }

    #[test]
    fn never_allows_more_than_capacity_all() {
        let clock = FakeRelativeClock::default();
        let lb = RateLimiter::direct_with_clock(Quota::per_second(4), &clock);
        let ms = Duration::from_millis(1);

        let num = NonZeroU32::new(2).unwrap();

        // Use up the burst capacity:
        assert!(lb.check_n(num).unwrap().is_ok());
        assert!(lb.check_n(num).unwrap().is_ok());

        clock.advance(ms);
        assert!(lb.check_n(num).unwrap().is_err());

        // should be ok again in 1s:
        clock.advance(ms * 1000);
        assert!(lb.check_n(num).unwrap().is_ok());
        clock.advance(ms);
        assert!(lb.check_n(num).unwrap().is_ok());

        clock.advance(ms);
        assert!(lb.check_n(num).unwrap().is_err(), "{lb:?}");
    }

    #[test]
    fn rejects_too_many_all() {
        let clock = FakeRelativeClock::default();
        let lb = RateLimiter::direct_with_clock(Quota::per_second(5), &clock);
        let ms = Duration::from_millis(1);

        let num = NonZeroU32::new(15).unwrap();

        // Should not allow the first 15 cells on a capacity 5 bucket:
        assert!(lb.check_n(num).is_err());

        // After 3 and 20 seconds, it should not allow 15 on that bucket either:
        clock.advance(ms * 3 * 1000);
        assert!(lb.check_n(num).is_err());
    }

    #[test]
    fn all_capacity_check_rejects_excess() {
        let clock = FakeRelativeClock::default();
        let lb = RateLimiter::direct_with_clock(Quota::per_second(5), &clock);

        assert_eq!(Err(InsufficientCapacity(5)), lb.check_n(NonZeroU32::new(15).unwrap()));
        assert_eq!(Err(InsufficientCapacity(5)), lb.check_n(NonZeroU32::new(6).unwrap()));
        assert_eq!(Err(InsufficientCapacity(5)), lb.check_n(NonZeroU32::new(7).unwrap()));
    }

    #[test]
    fn correct_wait_time() {
        let clock = FakeRelativeClock::default();
        // Bucket adding a new element per 200ms:
        let lb = RateLimiter::direct_with_clock(Quota::per_second(5), &clock);
        let ms = Duration::from_millis(1);
        let mut conforming = 0;
        for _i in 0..20 {
            clock.advance(ms);
            let res = lb.check();
            match res {
                Ok(_) => {
                    conforming += 1;
                }
                Err(wait) => {
                    clock.advance(wait.wait_time_from(clock.now()));
                    assert!(lb.check().is_ok());
                    conforming += 1;
                }
            }
        }
        assert_eq!(20, conforming);
    }

    #[test]
    fn actual_threadsafety() {
        use crossbeam;

        let clock = FakeRelativeClock::default();
        let lim = RateLimiter::direct_with_clock(Quota::per_second(20), &clock);
        let ms = Duration::from_millis(1);

        crossbeam::scope(|scope| {
            for _i in 0..20 {
                scope.spawn(|_| {
                    assert!(lim.check().is_ok());
                });
            }
        })
        .unwrap();

        clock.advance(ms * 2);
        assert!(lim.check().is_err());
        clock.advance(ms * 998);
        assert!(lim.check().is_ok());
    }

    #[test]
    fn default_direct() {
        let limiter = RateLimiter::direct_with_clock(Quota::per_second(20), &DefaultTimer);
        assert!(limiter.check().is_ok());
    }

    #[test]
    fn stresstest_large_quotas() {
        use std::{sync::Arc, thread};

        let quota = Quota::per_second(1_000_000_001);
        let rate_limiter = Arc::new(RateLimiter::direct(quota));

        fn rlspin(rl: Arc<DefaultDirectRateLimiter>) {
            for _ in 0..1_000_000 {
                rl.check().map_err(|e| dbg!(e)).unwrap();
            }
        }

        let rate_limiter2 = rate_limiter.clone();
        thread::spawn(move || {
            rlspin(rate_limiter2);
        });
        rlspin(rate_limiter);
    }

    const KEYS: &[u32] = &[1u32, 2u32];

    #[test]
    fn accepts_first_cell() {
        let clock = FakeRelativeClock::default();
        let lb = RateLimiter::hashmap_with_clock(Quota::per_second(5), &clock);
        for key in KEYS {
            assert!(lb.check_key(&key).is_ok(), "key {key:?}");
        }
    }

    use crate::state::keyed::HashMapStateStore;
    use core::hash::Hash;

    fn retained_keys<T: Clone + Hash + Eq + Copy + Ord>(
        limiter: RateLimiter<T, HashMapStateStore<T>, FakeRelativeClock>,
    ) -> Vec<T> {
        let state = limiter.into_state_store();
        let map = state.lock().unwrap();
        let mut keys: Vec<T> = map.keys().copied().collect();
        keys.sort();
        keys
    }

    #[test]
    fn expiration() {
        let clock = FakeRelativeClock::default();
        let ms = Duration::from_millis(1);

        let make_bucket = || {
            let lim = RateLimiter::hashmap_with_clock(Quota::per_second(1), &clock);
            lim.check_key(&"foo").unwrap();
            clock.advance(ms * 200);
            lim.check_key(&"bar").unwrap();
            clock.advance(ms * 600);
            lim.check_key(&"baz").unwrap();
            lim
        };
        let keys = &["bar", "baz", "foo"];

        // clean up all keys that are indistinguishable from unoccupied keys:
        let lim_shrunk = make_bucket();
        lim_shrunk.retain_recent();
        assert_eq!(retained_keys(lim_shrunk), keys);

        let lim_later = make_bucket();
        clock.advance(ms * 1200);
        lim_later.retain_recent();
        assert_eq!(retained_keys(lim_later), vec!["bar", "baz"]);

        let lim_later = make_bucket();
        clock.advance(ms * (1200 + 200));
        lim_later.retain_recent();
        assert_eq!(retained_keys(lim_later), vec!["baz"]);

        let lim_later = make_bucket();
        clock.advance(ms * (1200 + 200 + 600));
        lim_later.retain_recent();
        assert_eq!(retained_keys(lim_later), Vec::<&str>::new());
    }

    #[test]
    fn hashmap_length() {
        let lim = RateLimiter::hashmap(Quota::per_second(1));
        assert_eq!(lim.len(), 0);
        assert!(lim.is_empty());

        lim.check_key(&"foo").unwrap();
        assert_eq!(lim.len(), 1);
        assert!(!lim.is_empty(),);

        lim.check_key(&"bar").unwrap();
        assert_eq!(lim.len(), 2);
        assert!(!lim.is_empty());

        lim.check_key(&"baz").unwrap();
        assert_eq!(lim.len(), 3);
        assert!(!lim.is_empty());
    }

    #[test]
    fn hashmap_shrink_to_fit() {
        let clock = FakeRelativeClock::default();
        // a steady rate of 3ms between elements:
        let lim = RateLimiter::hashmap_with_clock(Quota::per_second(20), &clock);
        let ms = Duration::from_millis(1);

        assert!(lim
            .check_key_n(&"long-lived".to_string(), NonZeroU32::new(10).unwrap())
            .unwrap()
            .is_ok(),);
        assert!(lim.check_key(&"short-lived".to_string()).is_ok());

        // Move the clock forward far enough that the short-lived key gets dropped:
        clock.advance(ms * 300);
        lim.retain_recent();
        lim.shrink_to_fit();

        assert_eq!(lim.len(), 1);
    }

    fn resident_memory_size() -> i64 {
        let mut out: libc::rusage = unsafe { std::mem::zeroed() };
        assert!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut out) } == 0);
        out.ru_maxrss
    }

    const LEAK_TOLERANCE: i64 = 1024 * 1024 * 10;

    struct LeakCheck {
        usage_before: i64,
        n_iter: usize,
    }

    impl Drop for LeakCheck {
        fn drop(&mut self) {
            let usage_after = resident_memory_size();
            assert_le!(usage_after, self.usage_before + LEAK_TOLERANCE);
        }
    }

    impl LeakCheck {
        fn new(n_iter: usize) -> Self {
            LeakCheck {
                n_iter,
                usage_before: resident_memory_size(),
            }
        }
    }

    #[test]
    fn memleak_gcra() {
        let bucket = RateLimiter::direct(Quota::per_second(1_000_000));

        let leak_check = LeakCheck::new(500_000);

        for _i in 0..leak_check.n_iter {
            drop(bucket.check());
        }
    }

    #[test]
    fn memleak_gcra_multi() {
        let bucket = RateLimiter::direct(Quota::per_second(1_000_000));
        let leak_check = LeakCheck::new(500_000);

        for _i in 0..leak_check.n_iter {
            drop(bucket.check_n(NonZeroU32::new(2).unwrap()));
        }
    }

    #[test]
    fn memleak_gcra_threaded() {
        let bucket = Arc::new(RateLimiter::direct(Quota::per_second(1_000_000)));
        let leak_check = LeakCheck::new(5_000);

        for _i in 0..leak_check.n_iter {
            let bucket = Arc::clone(&bucket);
            thread::spawn(move || {
                assert!(bucket.check().is_ok());
            })
            .join()
            .unwrap();
        }
    }

    #[test]
    fn memleak_keyed() {
        let bucket = RateLimiter::keyed(Quota::per_second(50));

        let leak_check = LeakCheck::new(500_000);

        for i in 0..leak_check.n_iter {
            drop(bucket.check_key(&(i % 1000)));
        }
    }
}
