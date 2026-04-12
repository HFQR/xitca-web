use core::{
    hash::Hash,
    sync::atomic::{AtomicU64, Ordering},
};

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::Notify;

use super::Ready;

#[doc(hidden)]
pub struct Pool<K, C> {
    inner: Arc<Inner<K, C>>,
}

struct Inner<K, C> {
    conns: Mutex<HashMap<K, PooledConnection<C>>>,
    /// monotonic generation counter. each successfully cached entry is tagged
    /// with a unique generation so a stale acquirer (e.g. one whose probe
    /// returned `Err` after a fresh entry was already inserted) can recognize
    /// it must not evict the new entry.
    next_gen: AtomicU64,
}

impl<K, C> Clone for Pool<K, C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<K, C> Pool<K, C>
where
    K: Eq + Hash + Clone,
    C: Clone,
{
    pub(crate) fn with_capacity(_: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                conns: Mutex::new(HashMap::new()),
                next_gen: AtomicU64::new(0),
            }),
        }
    }

    pub(crate) async fn acquire(&self, key: impl Into<K>) -> AcquireOutput<'_, K, C>
    where
        C: Ready,
    {
        let key = key.into();
        loop {
            enum Step<C> {
                Cached { conn: C, generation: u64 },
                Wait(Arc<Notify>),
                Spawn(Arc<Notify>),
            }

            let step = {
                let mut conns = self.inner.conns.lock().unwrap();
                match conns.get(&key) {
                    Some(PooledConnection::Conn { conn, generation }) => Step::Cached {
                        conn: conn.clone(),
                        generation: *generation,
                    },
                    Some(PooledConnection::Spawning(notify)) => Step::Wait(notify.clone()),
                    None => {
                        let notify = Arc::new(Notify::new());
                        conns.insert(key.clone(), PooledConnection::Spawning(notify.clone()));
                        Step::Spawn(notify)
                    }
                }
            };

            match step {
                Step::Cached { mut conn, generation } => {
                    // probe the cached entry. when the underlying connection is no
                    // longer accepting new streams (e.g. peer sent GOAWAY) drop the
                    // entry from the cache and re-enter the loop to spawn or wait
                    // on a fresh one. the generation check makes sure we never
                    // evict a fresh entry that another acquirer installed while
                    // our probe was awaiting.
                    if conn.ready().await.is_err() {
                        evict_if_generation_matches(&self.inner.conns, &key, generation);
                        continue;
                    }
                    return AcquireOutput::Conn(Conn {
                        pool: self.clone(),
                        key,
                        conn,
                        generation,
                        destroy_on_drop: false,
                    });
                }
                Step::Wait(notify) => notify.notified().await,
                Step::Spawn(notify) => {
                    return AcquireOutput::Spawner(Spawner {
                        pool: self,
                        key,
                        notify,
                        fulfilled: false,
                    });
                }
            }
        }
    }
}

fn evict_if_generation_matches<K, C>(conns: &Mutex<HashMap<K, PooledConnection<C>>>, key: &K, generation: u64)
where
    K: Eq + Hash,
{
    let mut conns = conns.lock().unwrap();
    if let Some(PooledConnection::Conn {
        generation: current, ..
    }) = conns.get(key)
    {
        if *current == generation {
            conns.remove(key);
        }
    }
}

enum PooledConnection<C> {
    Conn { conn: C, generation: u64 },
    Spawning(Arc<Notify>),
}

pub(crate) enum AcquireOutput<'a, K, C>
where
    K: Eq + Hash + Clone,
{
    Conn(Conn<K, C>),
    Spawner(Spawner<'a, K, C>),
}

pub(crate) struct Conn<K, C>
where
    K: Eq + Hash + Clone,
{
    pool: Pool<K, C>,
    key: K,
    pub(crate) conn: C,
    /// generation of the cached entry this lease was cloned from. used to
    /// guard the destroy-on-drop eviction so a stale lease cannot remove a
    /// freshly spawned replacement.
    generation: u64,
    destroy_on_drop: bool,
}

pub(crate) struct Spawner<'a, K, C>
where
    K: Eq + Hash + Clone,
{
    pool: &'a Pool<K, C>,
    key: K,
    notify: Arc<Notify>,
    fulfilled: bool,
}

impl<K, C> Drop for Conn<K, C>
where
    K: Eq + Hash + Clone,
{
    fn drop(&mut self) {
        if self.destroy_on_drop {
            evict_if_generation_matches(&self.pool.inner.conns, &self.key, self.generation);
        }
    }
}

impl<K, C> Drop for Spawner<'_, K, C>
where
    K: Eq + Hash + Clone,
{
    fn drop(&mut self) {
        if !self.fulfilled {
            self.pool.inner.conns.lock().unwrap().remove(&self.key);
        }

        self.notify.notify_waiters();
    }
}

impl<K, C> Spawner<'_, K, C>
where
    K: Eq + Hash + Clone,
{
    pub(crate) fn spawned(mut self, conn: C) {
        self.fulfilled = true;
        let generation = self.pool.inner.next_gen.fetch_add(1, Ordering::Relaxed);
        if let Some(PooledConnection::Spawning(notify)) = self
            .pool
            .inner
            .conns
            .lock()
            .unwrap()
            .insert(self.key.clone(), PooledConnection::Conn { conn, generation })
        {
            notify.notify_waiters();
        }
    }
}

impl<K, C> Conn<K, C>
where
    K: Eq + Hash + Clone,
{
    pub(crate) fn set_destroy_on_drop(&mut self) {
        self.destroy_on_drop = true;
    }

    pub(crate) fn is_destroy_on_drop(&self) -> bool {
        self.destroy_on_drop
    }
}
