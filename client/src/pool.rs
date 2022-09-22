use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    ops::{Deref, DerefMut},
    sync::Mutex,
    time::{Duration, Instant},
};

use tokio::sync::{Semaphore, SemaphorePermit};

use crate::{connection::Multiplex, error::Error};

#[doc(hidden)]
pub struct Pool<K, C> {
    conns: Mutex<HashMap<K, Value<C>>>,
    permits: Semaphore,
}

enum Value<C> {
    Multiplexable(PooledConn<C>),
    NonMultiplexable(VecDeque<PooledConn<C>>),
}

impl<K, C> Pool<K, C>
where
    K: Eq + Hash + Clone,
    C: Multiplex,
{
    pub(crate) fn with_capacity(size: usize) -> Self {
        Self {
            conns: Mutex::new(HashMap::new()),
            permits: Semaphore::new(size),
        }
    }

    pub(crate) async fn acquire(&self, key: impl Into<K>) -> Result<Conn<'_, K, C>, Error> {
        // permit is needed to operate on pool.
        let permit = self.permits.acquire().await.unwrap();

        let key = key.into();

        let conn = {
            let mut conns = self.conns.lock().unwrap();

            let opt = conns.get_mut(&key);
            match opt {
                Some(Value::NonMultiplexable(queue)) => loop {
                    match queue.pop_front() {
                        // drop connection that are expired.
                        Some(conn) if conn.state.is_expired() => drop(conn),
                        conn => break conn,
                    }
                },
                Some(Value::Multiplexable(conn)) if conn.state.is_expired() => {
                    conns.remove(&key);
                    None
                }
                Some(Value::Multiplexable(conn)) => Some(conn.multiplex()),
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
    C: Multiplex,
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

    C: Multiplex,
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
    C: Multiplex,
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
    C: Multiplex,
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

    #[cfg(any(feature = "http1", feature = "http2", feature = "http3"))]
    pub(crate) fn destroy_on_drop(&mut self) {
        self.destroy_on_drop = true;
    }

    #[cfg(feature = "http1")]
    pub(crate) fn is_destroy_on_drop(&self) -> bool {
        self.destroy_on_drop
    }
}

impl<K, C> Drop for Conn<'_, K, C>
where
    K: Eq + Hash + Clone,
    C: Multiplex,
{
    fn drop(&mut self) {
        if let Some(mut conn) = self.conn.take() {
            if conn.state.is_expired() || self.destroy_on_drop {
                return;
            }

            let mut conns = self.pool.conns.lock().unwrap();

            let opt = conns.get_mut(&self.key);
            match opt {
                Some(Value::NonMultiplexable(queue)) => {
                    conn.state.update_idle();
                    queue.push_back(conn);
                }
                Some(Value::Multiplexable(conn)) => conn.state.update_idle(),
                None if conn.is_multiplexable() => {
                    conns.insert(self.key.clone(), Value::Multiplexable(conn));
                }
                None => {
                    let queue = VecDeque::from([conn]);
                    conns.insert(self.key.clone(), Value::NonMultiplexable(queue));
                }
            }

            let _ = self.permit;
        }
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

#[derive(Clone, Copy)]
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
        self.born.elapsed() > Duration::from_secs(3600) || self.idle_since.elapsed() > Duration::from_secs(600)
    }
}

impl<C: Multiplex> Multiplex for PooledConn<C> {
    fn multiplex(&mut self) -> Self {
        Self {
            conn: self.conn.multiplex(),
            state: self.state,
        }
    }

    fn is_multiplexable(&self) -> bool {
        self.conn.is_multiplexable()
    }
}
