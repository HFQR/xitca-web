//! State stores for rate limiters

pub mod direct;
pub mod keyed;

mod in_memory;

pub(crate) use self::{direct::*, in_memory::InMemoryState};

use crate::{
    gcra::Gcra,
    nanos::Nanos,
    quota::Quota,
    timer::{DefaultTimer, Timer},
};

/// A way for rate limiters to keep state.
///
/// There are two important kinds of state stores: Direct and keyed. The direct kind have only
/// one state, and are useful for "global" rate limit enforcement (e.g. a process should never
/// do more than N tasks a day). The keyed kind allows one rate limit per key (e.g. an API
/// call budget per client API key).
///
/// A direct state store is expressed as [`StateStore::Key`] = [`NotKeyed`].
/// Keyed state stores have a
/// type parameter for the key and set their key to that.
pub(crate) trait StateStore {
    /// The type of key that the state store can represent.
    type Key;

    /// Updates a state store's rate limiting state for a given key, using the given closure.
    ///
    /// The closure parameter takes the old value (`None` if this is the first measurement) of the
    /// state store at the key's location, checks if the request an be accommodated and:
    ///
    /// * If the request is rate-limited, returns `Err(E)`.
    /// * If the request can make it through, returns `Ok(T)` (an arbitrary positive return
    ///   value) and the updated state.
    ///
    /// It is `measure_and_replace`'s job then to safely replace the value at the key - it must
    /// only update the value if the value hasn't changed. The implementations in this
    /// crate use `AtomicU64` operations for this.
    fn measure_and_replace<T, F, E>(&self, key: &Self::Key, f: F) -> Result<T, E>
    where
        F: Fn(Option<Nanos>) -> Result<(T, Nanos), E>;
}

#[derive(Debug)]
pub(crate) struct RateLimiter<K, S, C = DefaultTimer>
where
    S: StateStore<Key = K>,
    C: Timer,
{
    state: S,
    gcra: Gcra,
    clock: C,
    start: C::Instant,
}

impl<K, S, C> RateLimiter<K, S, C>
where
    S: StateStore<Key = K>,
    C: Timer,
{
    pub(crate) fn new(quota: Quota, state: S, clock: &C) -> Self {
        let gcra = Gcra::new(quota);
        let start = clock.now();
        let clock = clock.clone();
        RateLimiter {
            state,
            clock,
            gcra,
            start,
        }
    }

    #[cfg(test)]
    pub(crate) fn into_state_store(self) -> S {
        self.state
    }
}
