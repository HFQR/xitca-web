use core::{
    hash::Hash,
    ops::{Deref, DerefMut},
    time::Duration,
};

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Instant,
};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

type Entries<K, C> = HashMap<K, (Arc<Semaphore>, VecDeque<PooledConn<C>>)>;

#[doc(hidden)]
pub struct Pool<K, C> {
    conns: Arc<Mutex<Entries<K, C>>>,
    // capacity for entry.
    // the pool can have unbounded entries with different keys but a single
    // entry can only have up to cap size of C inside it.
    cap: usize,
    keep_alive_idle: Duration,
    keep_alive_born: Duration,
    max_requests: usize,
}

impl<K, C> Clone for Pool<K, C> {
    fn clone(&self) -> Self {
        Self {
            conns: self.conns.clone(),
            cap: self.cap,
            keep_alive_idle: self.keep_alive_idle,
            keep_alive_born: self.keep_alive_born,
            max_requests: self.max_requests,
        }
    }
}

impl<K, C> Pool<K, C>
where
    K: Eq + Hash + Clone,
{
    pub(crate) fn new(cap: usize, keep_alive_idle: Duration, keep_alive_born: Duration, max_requests: usize) -> Self {
        Self {
            conns: Arc::new(Mutex::new(HashMap::new())),
            cap,
            keep_alive_idle,
            keep_alive_born,
            max_requests,
        }
    }

    // acquire a connection from pool. if a new connection needs to be made a spawner type
    // would be returned.
    pub(crate) async fn acquire(&self, key: impl Into<K>) -> AcquireOutput<'_, K, C> {
        let key = key.into();

        loop {
            let permits = {
                let mut conns = self.conns.lock().unwrap();
                match conns.get(&key) {
                    Some((permits, _)) => permits.clone(),
                    None => {
                        // no entry in pool. create new entry and return a spawner where a new connection
                        // can be spawned into the entry.
                        let permit = entry_new(&key, self.cap, &mut *conns);
                        return AcquireOutput::Spawner(Spawner {
                            pool: self,
                            key,
                            _permit: permit,
                            is_new_entry: true,
                            fulfilled: false,
                        });
                    }
                }
            };

            if let Ok(permit) = permits.acquire_owned().await {
                let mut conns = self.conns.lock().unwrap();
                let queue = match conns.get_mut(&key) {
                    Some((_, queue)) => queue,
                    // the entry is gone right after a permit is reserved.
                    // in this case try again from the beginning.
                    None => continue,
                };

                while let Some(conn) = queue.pop_front() {
                    if !conn.state.is_expired() {
                        return AcquireOutput::Conn(Conn {
                            pool: self.clone(),
                            key,
                            conn: Some(conn),
                            permit,
                            destroy_on_drop: false,
                        });
                    }
                }

                // all connection in entry are expired. in this case spawn new connection.
                return AcquireOutput::Spawner(Spawner {
                    pool: self,
                    key,
                    _permit: permit,
                    is_new_entry: false,
                    fulfilled: false,
                });
            }

            // the entry is gone when a permit is being reserved. in this case try again from the beginning.
        }
    }

    pub(crate) fn try_add(&self, key: impl Into<K>, conn: C) {
        let key = key.into();
        let mut conns = self.conns.lock().unwrap();
        match conns.get_mut(&key) {
            Some((permits, queue)) => {
                // try to acquire a permit immediately.
                // when failed the entry is already at full capacity. in that case just throw the connection.
                let res = permits.try_acquire();
                if res.is_ok() {
                    queue.push_back(PooledConn {
                        conn,
                        state: ConnState::new(self.keep_alive_idle, self.keep_alive_born, self.max_requests),
                    });
                }
            }
            None => {
                let permits = Arc::new(Semaphore::new(self.cap));
                let mut queue = VecDeque::with_capacity(self.cap);
                queue.push_back(PooledConn {
                    conn,
                    state: ConnState::new(self.keep_alive_idle, self.keep_alive_born, self.max_requests),
                });
                conns.insert(key, (permits, queue));
            }
        }
    }
}

// create new entry inside pool and reserve one permit immediately from the entry capacity.
fn entry_new<K, C>(key: &K, cap: usize, entries: &mut Entries<K, C>) -> OwnedSemaphorePermit
where
    K: Eq + Hash + Clone,
{
    let permits = Arc::new(Semaphore::new(cap));
    let permit = permits
        .clone()
        .try_acquire_owned()
        .expect("in place permit reservation must not fail");
    entries.insert(key.clone(), (permits, VecDeque::with_capacity(cap)));
    permit
}

pub enum AcquireOutput<'a, K, C>
where
    K: Eq + Hash + Clone,
{
    Conn(Conn<K, C>),
    Spawner(Spawner<'a, K, C>),
}

pub struct Conn<K, C>
where
    K: Eq + Hash + Clone,
{
    pool: Pool<K, C>,
    key: K,
    conn: Option<PooledConn<C>>,
    permit: OwnedSemaphorePermit,
    destroy_on_drop: bool,
}

impl<K, C> Deref for Conn<K, C>
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

impl<K, C> DerefMut for Conn<K, C>
where
    K: Eq + Hash + Clone,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.conn
            .as_deref_mut()
            .expect("DerefMut must be called when Conn::is_none returns false.")
    }
}

impl<K, C> Conn<K, C>
where
    K: Eq + Hash + Clone,
{
    #[cfg(feature = "http1")]
    pub(crate) fn destroy_on_drop(&mut self) {
        self.destroy_on_drop = true;
    }

    #[cfg(feature = "http1")]
    pub(crate) fn keep_alive_hint(&mut self, timeout: Option<Duration>, max_requests: Option<usize>) {
        if let Some(conn) = self.conn.as_mut() {
            if let Some(timeout) = timeout {
                conn.state.keep_alive_idle = timeout;
            }

            if let Some(max_requests) = max_requests {
                conn.state.max_requests = max_requests;
            }
        }
    }

    #[cfg(feature = "http1")]
    pub(crate) fn is_destroy_on_drop(&self) -> bool {
        self.destroy_on_drop
    }
}

impl<K, C> Drop for Conn<K, C>
where
    K: Eq + Hash + Clone,
{
    fn drop(&mut self) {
        if let Some(mut conn) = self.conn.take() {
            if conn.state.is_expired() || self.destroy_on_drop {
                return;
            }

            let mut conns = self.pool.conns.lock().unwrap();

            if let Some((_, queue)) = conns.get_mut(&self.key) {
                conn.state.update_for_reentry();
                queue.push_back(conn);
            }

            let _ = self.permit;
        }
    }
}

pub struct PooledConn<C> {
    conn: C,
    state: ConnState,
}

impl<K, C> From<Conn<K, C>> for PooledConn<C>
where
    K: Eq + Hash + Clone,
{
    fn from(mut conn: Conn<K, C>) -> Self {
        conn.conn.take().expect("Conn does not contain any connection")
    }
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

#[derive(Clone, Copy)]
struct ConnState {
    born: Instant,
    idle_since: Instant,
    requests: usize,
    keep_alive_idle: Duration,
    keep_alive_born: Duration,
    max_requests: usize,
}

impl ConnState {
    fn new(keep_alive_idle: Duration, keep_alive_born: Duration, max_requests: usize) -> Self {
        let now = Instant::now();

        Self {
            born: now,
            idle_since: now,
            requests: 0,
            keep_alive_idle,
            keep_alive_born,
            max_requests,
        }
    }

    fn update_for_reentry(&mut self) {
        self.idle_since = Instant::now();
        self.requests += 1;
    }

    fn is_expired(&self) -> bool {
        self.born.elapsed() > self.keep_alive_born
            || self.idle_since.elapsed() > self.keep_alive_idle
            || self.requests >= self.max_requests
    }
}

pub struct Spawner<'a, K, C>
where
    K: Eq + Hash + Clone,
{
    pool: &'a Pool<K, C>,
    key: K,
    _permit: OwnedSemaphorePermit,
    is_new_entry: bool,
    fulfilled: bool,
}

impl<K, C> Spawner<'_, K, C>
where
    K: Eq + Hash + Clone,
{
    pub(crate) fn spawned(mut self, conn: C) {
        self.fulfilled = true;

        if let Some((_, queue)) = self.pool.conns.lock().unwrap().get_mut(&self.key) {
            queue.push_back(PooledConn {
                conn,
                state: ConnState::new(
                    self.pool.keep_alive_idle,
                    self.pool.keep_alive_born,
                    self.pool.max_requests,
                ),
            });
        }
    }
}

impl<K, C> Drop for Spawner<'_, K, C>
where
    K: Eq + Hash + Clone,
{
    fn drop(&mut self) {
        if self.is_new_entry && !self.fulfilled {
            self.pool.conns.lock().unwrap().remove(&self.key);
        }
    }
}
