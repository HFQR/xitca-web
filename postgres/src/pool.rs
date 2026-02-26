mod connect;
mod connection;
mod execute;

pub use self::connect::Connect;
pub use self::connection::{CachedStatement, PoolConnection};

use core::num::NonZeroUsize;

use std::sync::Arc;

use tokio::sync::Semaphore;

use super::{config::Config, error::Error};

use self::{
    _pool::{_Pool, Permit, PermitOwned},
    connect::{ConnectorDyn, DefaultConnector},
    connection::PoolClient,
};

/// builder type for connection pool
pub struct PoolBuilder {
    config: Result<Config, Error>,
    capacity: usize,
    cache_size: usize,
    connector: Box<dyn ConnectorDyn>,
}

impl PoolBuilder {
    /// set capacity. pool would spawn up to amount of capacity concurrent connections to database.
    ///
    /// # Default
    /// capacity default to 1
    pub fn capacity(mut self, cap: usize) -> Self {
        self.capacity = cap;
        self
    }

    /// set statment cache size. every connection in pool would keep a set of prepared statement cache to
    /// speed up repeated query
    ///
    /// # Default
    /// cache_size default to 16
    pub fn cache_size(mut self, size: usize) -> Self {
        self.cache_size = size;
        self
    }

    /// set connector type for establishing connection to database. C must impl [`Connect`] trait
    pub fn connector<C>(mut self, connector: C) -> Self
    where
        C: Connect + 'static,
    {
        self.connector = Box::new(connector) as _;
        self
    }

    /// try convert builder to a connection pool instance.
    pub fn build(self) -> Result<Pool, Error> {
        let cfg = self.config?;
        let cache_size = NonZeroUsize::new(self.cache_size).ok_or_else(Error::todo)?;
        Ok(Pool {
            pool: _Pool::new(self.capacity, cache_size, cfg, self.connector),
            permits: Semaphore::new(self.capacity),
        })
    }

    /// try convert builder to a connection pool instance.
    pub fn build_owned(self) -> Result<PoolOwned, Error> {
        self.build().map(|pool| PoolOwned {
            pool: Arc::new(pool.pool),
            permits: Arc::new(pool.permits),
        })
    }
}

/// connection pool for a set of connections to database. Can be used as entry point of query
///
/// # Examples
/// ```rust
/// # use xitca_postgres::{Error, Execute};
/// # async fn query() -> Result<(), Error> {
/// let pool = xitca_postgres::pool::Pool::builder("db_url").build()?;
/// xitca_postgres::Statement::named("SELECT 1", &[]).bind_none().execute(&pool).await?;
/// # Ok(())
/// # }
/// ```
///
/// # Caching
/// When connection pool is used as executor through [`Execute::query`] and [`Execute::execute`] methods
/// it would prepare and cache statement for reuse. For selective caching consider use [`PoolConnection`]
///
/// [`Execute::query`]: crate::Execute::query
/// [`Execute::execute`]: crate::Execute::execute
pub struct Pool {
    pool: _Pool,
    permits: Semaphore,
}

impl Pool {
    /// start a builder of pool where it's behavior can be configured.
    pub fn builder<C>(cfg: C) -> PoolBuilder
    where
        Config: TryFrom<C>,
        Error: From<<Config as TryFrom<C>>::Error>,
    {
        PoolBuilder {
            config: cfg.try_into().map_err(Into::into),
            capacity: 1,
            cache_size: 16,
            connector: Box::new(DefaultConnector),
        }
    }

    /// try to get a connection from pool.
    /// when pool is empty it will try to spawn new connection to database and if the process failed the outcome will
    /// return as [`Error`]
    pub async fn get(&self) -> Result<PoolConnection<Permit<'_>>, Error> {
        let _permit = self.permits.acquire().await.expect("Semaphore must not be closed");

        let conn = match self.pool.try_get() {
            Some(conn) => conn,
            None => self.pool.connect().await?,
        };

        Ok(PoolConnection {
            conn: Some(conn),
            pool_ref: Permit {
                pool: &self.pool,
                _permit,
            },
        })
    }

    /// get configration of current connection pool.
    pub fn config(&self) -> &Config {
        self.pool.config()
    }
}

/// an owned version of connection pool
///
/// function the same as [`Pool`] while offering relaxed lifetime params
/// can be cheaply cloned
#[derive(Clone)]
pub struct PoolOwned {
    pool: Arc<_Pool>,
    permits: Arc<Semaphore>,
}

impl PoolOwned {
    /// try to get a connection from pool.
    /// when pool is empty it will try to spawn new connection to database and if the process failed the outcome will
    /// return as [`Error`]
    pub async fn get(&self) -> Result<PoolConnection<PermitOwned>, Error> {
        let _permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .expect("Semaphore must not be closed");

        let conn = match self.pool.try_get() {
            Some(conn) => conn,
            None => self.pool.connect().await?,
        };

        Ok(PoolConnection {
            conn: Some(conn),
            pool_ref: PermitOwned {
                pool: self.pool.clone(),
                _permit,
            },
        })
    }

    /// get configration of current connection pool.
    pub fn config(&self) -> &Config {
        self.pool.config()
    }
}

/// trait generic over different type of permition from connection pool
pub trait PermitLike: Send + _pool::Sealed {
    fn put_back(&self, conn: PoolClient);
}

mod _pool {
    use std::{collections::VecDeque, sync::Mutex};

    use tokio::sync::{OwnedSemaphorePermit, SemaphorePermit};

    use super::{Arc, Config, ConnectorDyn, Error, NonZeroUsize, PermitLike, PoolClient};

    pub struct _Pool {
        pub(super) conn: Mutex<VecDeque<PoolClient>>,
        config: Box<PoolConfig>,
    }

    struct PoolConfig {
        connector: Box<dyn ConnectorDyn>,
        cfg: Config,
        cache_size: NonZeroUsize,
    }

    impl _Pool {
        pub(super) fn new(cap: usize, cache_size: NonZeroUsize, cfg: Config, connector: Box<dyn ConnectorDyn>) -> Self {
            Self {
                conn: Mutex::new(VecDeque::with_capacity(cap)),
                config: Box::new(PoolConfig {
                    connector,
                    cfg,
                    cache_size,
                }),
            }
        }

        pub(super) fn config(&self) -> &Config {
            &self.config.cfg
        }

        pub(super) fn try_get(&self) -> Option<PoolClient> {
            let mut inner = self.conn.lock().unwrap();

            while let Some(conn) = inner.pop_front() {
                if !conn.closed() {
                    return Some(conn);
                }
            }

            None
        }

        fn put_back(&self, conn: PoolClient) {
            self.conn.lock().unwrap().push_back(conn);
        }

        pub(super) async fn connect(&self) -> Result<PoolClient, Error> {
            self.config
                .connector
                .connect_dyn(self.config.cfg.clone())
                .await
                .map(|cli| PoolClient::new(cli, self.config.cache_size))
        }
    }

    pub trait Sealed {}

    pub struct Permit<'a> {
        pub(super) pool: &'a _Pool,
        pub(super) _permit: SemaphorePermit<'a>,
    }

    impl Sealed for Permit<'_> {}

    impl PermitLike for Permit<'_> {
        #[inline]
        fn put_back(&self, conn: PoolClient) {
            self.pool.put_back(conn);
        }
    }

    pub struct PermitOwned {
        pub(super) pool: Arc<_Pool>,
        pub(super) _permit: OwnedSemaphorePermit,
    }

    impl Sealed for PermitOwned {}

    impl PermitLike for PermitOwned {
        #[inline]
        fn put_back(&self, conn: PoolClient) {
            self.pool.put_back(conn);
        }
    }
}

#[cfg(not(feature = "io-uring"))]
#[cfg(test)]
mod test {
    use crate::{execute::Execute, iter::AsyncLendingIterator, statement::Statement};

    use super::*;

    #[tokio::test]
    async fn pool() {
        let pool = Pool::builder("postgres://postgres:postgres@localhost:5432")
            .build()
            .unwrap();

        {
            let mut conn = pool.get().await.unwrap();

            let stmt = Statement::named("SELECT 1", &[]).execute(&mut conn).await.unwrap();
            stmt.execute(&conn.consume()).await.unwrap();

            let num = Statement::named("SELECT 1", &[])
                .bind_none()
                .query(&pool)
                .await
                .unwrap()
                .try_next()
                .await
                .unwrap()
                .unwrap()
                .get::<i32>(0);

            assert_eq!(num, 1);
        }

        let res = [
            Statement::named("SELECT 1", &[]).bind_none(),
            Statement::named("SELECT 1", &[]).bind_none(),
        ]
        .query(&pool)
        .await
        .unwrap();

        for mut res in res {
            let num = res.try_next().await.unwrap().unwrap().get::<i32>(0);
            assert_eq!(num, 1);
        }

        let _ = vec![
            Statement::named("SELECT 1", &[]).bind_dyn(&[&1]),
            Statement::named("SELECT 1", &[]).bind_dyn(&[&"123"]),
            Statement::named("SELECT 1", &[]).bind_dyn(&[&String::new()]),
        ]
        .query(&pool)
        .await;
    }
}
