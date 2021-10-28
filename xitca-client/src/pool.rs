use std::{
    collections::VecDeque,
    hash::Hash,
    ops::{Deref, DerefMut},
    time::{Duration, Instant},
};

use ahash::AHashMap;
use parking_lot::Mutex;
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::error::Error;

#[doc(hidden)]
pub struct Pool<K, C> {
    conns: Mutex<AHashMap<K, VecDeque<PooledConn<C>>>>,
    permits: Semaphore,
}

impl<K, C> Pool<K, C>
where
    K: Eq + Hash + Clone,
{
    pub(crate) fn with_capacity(size: usize) -> Self {
        Self {
            conns: Mutex::new(AHashMap::new()),
            permits: Semaphore::new(size),
        }
    }

    pub(crate) async fn acquire(&self, key: impl Into<K>) -> Result<Conn<'_, K, C>, Error> {
        // permit is needed to operate on pool.
        let permit = self.permits.acquire().await.unwrap();

        let key = key.into();

        let conn = {
            let mut conns = self.conns.lock();

            match conns.get_mut(&key) {
                Some(queue) => loop {
                    match queue.pop_front() {
                        // drop connection that are expired.
                        Some(conn) if conn.state.is_expired() => drop(conn),
                        conn => break conn,
                    }
                },
                None => None,
            }
        };

        Ok(Conn {
            pool: self,
            key,
            conn,
            permit,
            destroy_on_drop: false,
        })
    }
}

pub struct Conn<'a, K, C>
where
    K: Eq + Hash + Clone,
{
    pool: &'a Pool<K, C>,
    key: K,
    conn: Option<PooledConn<C>>,
    permit: SemaphorePermit<'a>,
    destroy_on_drop: bool,
}

impl<K, C> Deref for Conn<'_, K, C>
where
    K: Eq + Hash + Clone,
{
    type Target = C;

    fn deref(&self) -> &Self::Target {
        self.conn
            .as_deref()
            .expect("Deref must be called when Conn::is_none returns false.")
    }
}

impl<K, C> DerefMut for Conn<'_, K, C>
where
    K: Eq + Hash + Clone,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.conn
            .as_deref_mut()
            .expect("DerefMut must be called when Conn::is_none returns false.")
    }
}

impl<K, C> Conn<'_, K, C>
where
    K: Eq + Hash + Clone,
{
    pub(crate) fn is_none(&self) -> bool {
        self.conn.is_none()
    }

    pub(crate) fn add(&mut self, conn: C) {
        debug_assert!(self.is_none());
        self.conn = Some(PooledConn {
            conn,
            state: ConnState::new(),
        });
    }

    pub(crate) fn destroy_on_drop(&mut self) {
        self.destroy_on_drop = true;
    }
}

impl<K, C> Drop for Conn<'_, K, C>
where
    K: Eq + Hash + Clone,
{
    fn drop(&mut self) {
        let mut conn = self.conn.take().unwrap();

        if conn.state.is_expired() || self.destroy_on_drop {
            return;
        }

        conn.state.update_idle();

        let mut conns = self.pool.conns.lock();

        match conns.get_mut(&self.key) {
            Some(queue) => queue.push_back(conn),
            None => {
                let queue = VecDeque::from([conn]);
                conns.insert(self.key.clone(), queue);
            }
        }

        let _ = self.permit;
    }
}

struct PooledConn<C> {
    conn: C,
    state: ConnState,
}

impl<C> Deref for PooledConn<C> {
    type Target = C;

    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}

impl<C> DerefMut for PooledConn<C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.conn
    }
}

struct ConnState {
    born: Instant,
    idle_since: Instant,
}

impl ConnState {
    fn new() -> Self {
        let now = Instant::now();

        Self {
            born: now,
            idle_since: now,
        }
    }

    fn update_idle(&mut self) {
        self.idle_since = Instant::now();
    }

    fn is_expired(&self) -> bool {
        self.born.elapsed() > Duration::from_secs(3600) || self.idle_since.elapsed() > Duration::from_secs(60)
    }
}
