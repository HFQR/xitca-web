use core::hash::Hash;

use crate::{gcra::NotUntil, quota::Quota, snapshot::RateSnapshot, state::RateLimiter, state::StateStore, timer};

#[cfg(test)]
use core::num::NonZeroU32;

#[cfg(test)]
use crate::{error::InsufficientCapacity, timer::Reference};

// A trait for state stores with one rate limiting state per key.
//
// This is blanket-implemented by all `StateStore`s with hashable `Eq + Hash + Clone` key
// associated types.
pub(crate) trait KeyedStateStore<K: Hash>: StateStore<Key = K> {}

impl<T, K: Hash> KeyedStateStore<K> for T
where
    T: StateStore<Key = K>,
    K: Eq + Clone + Hash,
{
}

/// # Keyed rate limiters - default constructors
impl<K> RateLimiter<K, DefaultKeyedStateStore<K>, timer::DefaultTimer>
where
    K: Clone + Hash + Eq,
{
    #[cfg(test)]
    // Constructs a new keyed rate limiter backed by
    // the [`DefaultKeyedStateStore`].
    pub(crate) fn keyed(quota: Quota) -> Self {
        let state = DefaultKeyedStateStore::default();
        let clock = timer::DefaultTimer;
        RateLimiter::new(quota, state, &clock)
    }

    /// Constructs a new keyed rate limiter explicitly backed by a
    /// [`HashMap`][std::collections::HashMap].
    pub(crate) fn hashmap(quota: Quota) -> Self {
        let state = HashMapStateStore::default();
        let timer = timer::DefaultTimer;
        RateLimiter::new(quota, state, &timer)
    }
}

/// # Keyed rate limiters - Manually checking cells
impl<K, S, C> RateLimiter<K, S, C>
where
    S: KeyedStateStore<K>,
    K: Hash,
    C: timer::Timer,
{
    /// Allow a single cell through the rate limiter for the given key.
    ///
    /// If the rate limit is reached, `check_key` returns information about the earliest
    /// time that a cell might be allowed through again under that key.
    pub fn check_key(&self, key: &K) -> Result<RateSnapshot, NotUntil<C::Instant>> {
        self.gcra
            .test_and_update::<K, C::Instant, S>(self.start, key, &self.state, self.clock.now())
    }

    #[cfg(test)]
    /// Allow *only all* `n` cells through the rate limiter for the given key.
    ///
    /// This method can succeed in only one way and fail in two ways:
    /// * Success: If all `n` cells can be accommodated, it returns `Ok(Ok(()))`.
    /// * Failure (but ok): Not all cells can make it through at the current time.
    ///   The result is `Ok(Err(NotUntil))`, which can
    ///   be interrogated about when the batch might next conform.
    /// * Failure (the batch can never go through): The rate limit is too low for the given number
    ///   of cells. The result is `Err(InsufficientCapacity)`
    ///
    /// ### Performance
    /// This method diverges a little from the GCRA algorithm, using
    /// multiplication to determine the next theoretical arrival time, and so
    /// is not as fast as checking a single cell.
    pub(crate) fn check_key_n(
        &self,
        key: &K,
        n: NonZeroU32,
    ) -> Result<Result<RateSnapshot, NotUntil<C::Instant>>, InsufficientCapacity> {
        self.gcra
            .test_n_all_and_update::<K, C::Instant, S>(self.start, key, n, &self.state, self.clock.now())
    }
}

#[cfg(test)]
/// Keyed rate limiters that can be "cleaned up".
///
/// Any keyed state store implementing this trait allows users to evict elements that are
/// indistinguishable from fresh rate-limiting states (that is, if a key hasn't been used for
/// rate-limiting decisions for as long as the bucket capacity).
///
/// As this does not make sense for not all keyed state stores (e.g. stores that auto-expire like
/// memcache), this is an optional trait. All the keyed state stores in this crate implement
/// shrinking.
pub(crate) trait ShrinkableKeyedStateStore<K: Hash>: KeyedStateStore<K> {
    /// Remove those keys with state older than `drop_below`.
    fn retain_recent(&self, drop_below: crate::nanos::Nanos);

    /// Shrinks the capacity of the state store, if possible.
    ///
    /// If the state store does not support shrinking, this method is a no-op.
    fn shrink_to_fit(&self) {}

    /// Returns the number of "live" keys stored in the state store.
    ///
    /// Depending on how the state store is implemented, this may
    /// return an estimate or an out-of-date result.
    fn len(&self) -> usize;

    /// Returns `true` if `self` has no keys stored in it.
    ///
    /// As with [`len`](#tymethod.len), this method may return
    /// imprecise results (indicating that the state store is empty
    /// while a concurrent rate-limiting operation is taking place).
    fn is_empty(&self) -> bool;
}

#[cfg(test)]
/// # Keyed rate limiters - Housekeeping
///
/// As the inputs to a keyed rate-limiter can be arbitrary keys, the set of retained keys retained
/// grows, while the number of active keys may stay smaller. To save on space, a keyed rate-limiter
/// allows removing those keys that are "stale", i.e., whose values are no different from keys' that
/// aren't present in the rate limiter state store.
impl<K, S, C> RateLimiter<K, S, C>
where
    S: ShrinkableKeyedStateStore<K>,
    K: Hash,
    C: timer::Timer,
{
    // Retains all keys in the rate limiter that were used recently enough.
    //
    // Any key whose rate limiting state is indistinguishable from a "fresh" state (i.e., the
    // theoretical arrival time lies in the past).
    pub(crate) fn retain_recent(&self) {
        // calculate the minimum retention parameter: Any key whose state store's theoretical
        // arrival time is larger than a starting state for the bucket gets to stay, everything
        // else (that's indistinguishable from a starting state) goes.
        let now = self.clock.now();
        let drop_below = now.duration_since(self.start);

        self.state.retain_recent(drop_below);
    }

    // Shrinks the capacity of the rate limiter's state store, if possible.
    pub(crate) fn shrink_to_fit(&self) {
        self.state.shrink_to_fit();
    }

    // Returns the number of "live" keys in the rate limiter's state store.
    //
    // Depending on how the state store is implemented, this may
    // return an estimate or an out-of-date result.
    pub(crate) fn len(&self) -> usize {
        self.state.len()
    }

    // Returns `true` if the rate limiter has no keys in it.
    //
    // As with [`len`](#method.len), this method may return
    // imprecise results (indicating that the state store is empty
    // while a concurrent rate-limiting operation is taking place).
    pub(crate) fn is_empty(&self) -> bool {
        self.state.is_empty()
    }
}

mod hashmap;

pub(crate) use hashmap::HashMapStateStore;

pub(crate) type DefaultKeyedStateStore<K> = HashMapStateStore<K>;

#[cfg(test)]
mod test {
    use core::marker::PhantomData;

    use crate::{nanos::Nanos, timer::FakeRelativeClock};

    use super::*;

    #[test]
    fn default_nonshrinkable_state_store_coverage() {
        #[derive(Default)]
        struct NaiveKeyedStateStore<K>(PhantomData<K>);

        impl<K: Hash + Eq + Clone> StateStore for NaiveKeyedStateStore<K> {
            type Key = K;

            fn measure_and_replace<T, F, E>(&self, _key: &Self::Key, f: F) -> Result<T, E>
            where
                F: Fn(Option<Nanos>) -> Result<(T, Nanos), E>,
            {
                f(None).map(|(res, _)| res)
            }
        }

        impl<K: Hash + Eq + Clone> ShrinkableKeyedStateStore<K> for NaiveKeyedStateStore<K> {
            fn retain_recent(&self, _drop_below: Nanos) {
                // nothing to do
            }

            fn len(&self) -> usize {
                0
            }
            fn is_empty(&self) -> bool {
                true
            }
        }

        let lim: RateLimiter<u32, NaiveKeyedStateStore<u32>, FakeRelativeClock> = RateLimiter::new(
            Quota::per_second(NonZeroU32::new(1).unwrap()),
            NaiveKeyedStateStore::default(),
            &FakeRelativeClock::default(),
        );
        assert!(lim.check_key(&1u32).is_ok());
        assert!(lim.is_empty());
        assert_eq!(lim.len(), 0);
        lim.retain_recent();
        lim.shrink_to_fit();
    }
}
