use std::{collections::VecDeque, hash::Hash};

use ahash::AHashMap;
use parking_lot::Mutex;
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::error::Error;

#[doc(hidden)]
pub struct Pool<K, C> {
    conns: Mutex<AHashMap<K, VecDeque<C>>>,
    permits: Semaphore,
}

impl<K, C> Pool<K, C>
where
    K: Eq + Hash,
{
    pub(crate) fn with_capacity(size: usize) -> Self {
        Self {
            conns: Mutex::new(AHashMap::new()),
            permits: Semaphore::new(size),
        }
    }

    pub(crate) async fn acquire(&self, key: K) -> Result<PoolConnection<'_, K, C>, Error> {
        let permit = self.permits.acquire().await.unwrap();

        let conn = {
            let mut conns = self.conns.lock();

            match conns.get_mut(&key) {
                Some(queue) => queue.pop_front(),
                None => None,
            }
        };

        Ok(PoolConnection {
            pool: self,
            key: Some(key),
            conn,
            permit,
        })
    }
}

pub struct PoolConnection<'a, K, C>
where
    K: Eq + Hash,
{
    pool: &'a Pool<K, C>,
    key: Option<K>,
    conn: Option<C>,
    permit: SemaphorePermit<'a>,
}

impl<K, C> PoolConnection<'_, K, C>
where
    K: Eq + Hash,
{
    pub(crate) fn is_none(&self) -> bool {
        self.conn.is_none()
    }

    pub(crate) fn add_conn(&mut self, conn: C) {
        debug_assert!(self.is_none());
        self.conn = Some(conn);
    }
}

impl<K, C> Drop for PoolConnection<'_, K, C>
where
    K: Eq + Hash,
{
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let key = self.key.take().unwrap();
            self.pool
                .conns
                .lock()
                .entry(key)
                .or_insert_with(VecDeque::new)
                .push_back(conn);

            let _ = self.permit;
        }
    }
}
