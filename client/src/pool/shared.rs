use core::hash::Hash;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::Notify;

#[doc(hidden)]
pub struct Pool<K, C> {
    conns: Mutex<HashMap<K, Connection<C>>>,
}

impl<K, C> Pool<K, C>
where
    K: Eq + Hash + Clone,
    C: Clone,
{
    pub(crate) fn with_capacity(_: usize) -> Self {
        Self {
            conns: Mutex::new(HashMap::new()),
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
                    Some(Connection::Conn(c)) => return AcquireOutput::Conn(c.clone()),
                    Some(Connection::Spawning(notify)) => notify.clone(),
                    None => {
                        let notify = Arc::new(Notify::new());
                        conns.insert(key.clone(), Connection::Spawning(notify.clone()));
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

enum Connection<C> {
    Conn(C),
    Spawning(Arc<Notify>),
}

pub(crate) enum AcquireOutput<'a, K, C>
where
    K: Eq + Hash + Clone,
{
    Conn(C),
    Spawner(Spawner<'a, K, C>),
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
        if let Some(Connection::Spawning(notify)) = self
            .pool
            .conns
            .lock()
            .unwrap()
            .insert(self.key.clone(), Connection::Conn(conn))
        {
            notify.notify_waiters();
        }
    }
}
