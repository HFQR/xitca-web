use core::hash::Hash;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::Notify;

#[doc(hidden)]
pub struct Pool<K, C> {
    conns: Arc<Mutex<HashMap<K, PooledConnection<C>>>>,
}

impl<K, C> Clone for Pool<K, C> {
    fn clone(&self) -> Self {
        Self {
            conns: self.conns.clone(),
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
            conns: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn acquire(&self, key: impl Into<K>) -> AcquireOutput<'_, K, C>
    where
        C: Clone,
    {
        let key = key.into();
        loop {
            let notify = {
                let mut conns = self.conns.lock().unwrap();
                match conns.get(&key) {
                    Some(PooledConnection::Conn(c)) => {
                        return AcquireOutput::Conn(Conn {
                            pool: self.clone(),
                            key,
                            conn: c.clone(),
                            destroy_on_drop: false,
                        });
                    }
                    Some(PooledConnection::Spawning(notify)) => notify.clone(),
                    None => {
                        let notify = Arc::new(Notify::new());
                        conns.insert(key.clone(), PooledConnection::Spawning(notify.clone()));
                        return AcquireOutput::Spawner(Spawner {
                            pool: self,
                            key,
                            notify,
                            fulfilled: false,
                        });
                    }
                }
            };
            notify.notified().await;
        }
    }
}

enum PooledConnection<C> {
    Conn(C),
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
            let mut conns = self.pool.conns.lock().unwrap();
            if matches!(conns.get(&self.key), Some(PooledConnection::Conn(_))) {
                conns.remove(&self.key);
            }
        }
    }
}

impl<K, C> Drop for Spawner<'_, K, C>
where
    K: Eq + Hash + Clone,
{
    fn drop(&mut self) {
        if !self.fulfilled {
            self.pool.conns.lock().unwrap().remove(&self.key);
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
        if let Some(PooledConnection::Spawning(notify)) = self
            .pool
            .conns
            .lock()
            .unwrap()
            .insert(self.key.clone(), PooledConnection::Conn(conn))
        {
            notify.notify_waiters();
        }
    }
}

impl<K, C> Conn<K, C>
where
    K: Eq + Hash + Clone,
{
    pub(crate) fn destroy_on_drop(&mut self) {
        self.destroy_on_drop = true;
    }
}
