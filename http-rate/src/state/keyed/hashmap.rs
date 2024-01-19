use core::hash::Hash;

use std::{collections::HashMap, sync::Mutex};

use crate::{
    nanos::Nanos,
    state::{keyed::ShrinkableKeyedStateStore, InMemoryState, StateStore},
};

#[cfg(test)]
use crate::{quota::Quota, state::RateLimiter, timer::Timer};

/// A thread-safe (but not very performant) implementation of a keyed rate limiter state
/// store using [`HashMap`].
///
/// The `HashMapStateStore` is the default state store in `std` when no other thread-safe
/// features are enabled.
pub(crate) type HashMapStateStore<K> = Mutex<HashMap<K, InMemoryState>>;

impl<K> StateStore for HashMapStateStore<K>
where
    K: Hash + Eq + Clone,
{
    type Key = K;

    fn measure_and_replace<T, F, E>(&self, key: &Self::Key, f: F) -> Result<T, E>
    where
        F: Fn(Option<Nanos>) -> Result<(T, Nanos), E>,
    {
        let mut map = self.lock().unwrap();
        if let Some(v) = (*map).get(key) {
            // fast path: a rate limiter is already present for the key.
            return v.measure_and_replace_one(f);
        }
        // not-so-fast path: make a new entry and measure it.
        let entry = (*map).entry(key.clone()).or_default();
        entry.measure_and_replace_one(f)
    }
}

impl<K> ShrinkableKeyedStateStore<K> for HashMapStateStore<K>
where
    K: Hash + Eq + Clone,
{
    fn retain_recent(&self, drop_below: Nanos) {
        let mut map = self.lock().unwrap();
        map.retain(|_, v| !v.is_older_than(drop_below));
    }

    fn shrink_to_fit(&self) {
        let mut map = self.lock().unwrap();
        map.shrink_to_fit();
    }

    fn len(&self) -> usize {
        let map = self.lock().unwrap();
        (*map).len()
    }
    fn is_empty(&self) -> bool {
        let map = self.lock().unwrap();
        (*map).is_empty()
    }
}

#[cfg(test)]
impl<K, C> RateLimiter<K, HashMapStateStore<K>, C>
where
    K: Hash + Eq + Clone,
    C: Timer,
{
    /// Constructs a new rate limiter with a custom clock, backed by a [`HashMap`].
    pub(crate) fn hashmap_with_clock(quota: Quota, clock: &C) -> Self {
        let state = Mutex::new(HashMap::new());
        RateLimiter::new(quota, state, clock)
    }
}
