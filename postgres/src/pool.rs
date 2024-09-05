use core::{
    future::{Future, IntoFuture},
    ops::DerefMut,
};

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use tokio::sync::{Semaphore, SemaphorePermit};
use xitca_io::bytes::BytesMut;

use super::{
    client::Client,
    config::Config,
    error::DriverDown,
    error::Error,
    iter::slice_iter,
    pipeline::{Pipeline, PipelineStream},
    statement::Statement,
    BorrowToSql, Postgres, RowStream, ToSql, Type,
};

/// builder type for connection pool
pub struct PoolBuilder {
    config: Result<Config, Error>,
    capacity: usize,
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

    /// try convert builder to a connection pool instance.
    pub fn build(self) -> Result<Pool, Error> {
        let config = self.config?;

        Ok(Pool {
            conn: Mutex::new(VecDeque::with_capacity(self.capacity)),
            permits: Semaphore::new(self.capacity),
            config,
        })
    }
}

/// connection pool for a set of connections to database.
pub struct Pool {
    conn: Mutex<VecDeque<PoolClient>>,
    permits: Semaphore,
    config: Config,
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
        }
    }

    /// try to get a connection from pool.
    /// when pool is empty it will try to spawn new connection to database and if the process failed the outcome will
    /// return as [Error]
    pub async fn get(&self) -> Result<PoolConnection<'_>, Error> {
        let _permit = self.permits.acquire().await.expect("Semaphore must not be closed");
        let conn = self.conn.lock().unwrap().pop_front();
        match conn {
            Some(conn) => Ok(PoolConnection {
                pool: self,
                conn: Some((conn, _permit)),
            }),
            None => {
                let (client, driver) = Postgres::new(self.config.clone()).connect().await?;

                tokio::task::spawn(driver.into_future());

                Ok(PoolConnection {
                    pool: self,
                    conn: Some((PoolClient::new(client), _permit)),
                })
            }
        }
    }
}

/// a RAII type for connection. it manages the lifetime of connection and it's [Statement] cache.
/// a set of public is exposed to interact with them.
pub struct PoolConnection<'a> {
    pool: &'a Pool,
    conn: Option<(PoolClient, SemaphorePermit<'a>)>,
}

impl<'p> PoolConnection<'p> {
    /// prepare a [Statement] for future query. statement will be cached for future reuse and the
    /// return type is [Arc<Statement>] smart pointer. this smart pointer is used to detach ownership
    /// from [ExclusiveConnection]
    pub async fn prepare(&mut self, query: &str, types: &[Type]) -> Result<Arc<Statement>, Error> {
        match self.try_conn()?.statements.get(query) {
            Some(stmt) => Ok(stmt.clone()),
            None => {
                let stmt = self
                    .try_conn()
                    .unwrap()
                    .client
                    .prepare(query, types)
                    .await
                    .map(|stmt| Arc::new(stmt.leak()))
                    .inspect_err(|e| self.try_drop_on_error(e))?;
                self.try_conn()
                    .unwrap()
                    .statements
                    .insert(Box::from(query), stmt.clone());
                Ok(stmt)
            }
        }
    }

    /// function the same as [Client::query]
    #[inline]
    pub fn query<'a>(&mut self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.query_raw(stmt, slice_iter(params))
    }

    /// function the same as [Client::query_raw]
    pub fn query_raw<'a, I>(&mut self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator + Clone,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.try_conn()?
            .client
            .query_raw(stmt, params)
            .inspect_err(|e| self.try_drop_on_error(e))
    }

    /// function the same as [Client::execute_simple]
    pub fn execute_simple(&mut self, stmt: &str) -> impl Future<Output = Result<u64, Error>> + Send {
        let res = match self.try_conn() {
            Ok(conn) => conn
                .client
                .send_encode_simple(stmt)
                .inspect_err(|e| self.try_drop_on_error(e)),
            Err(e) => Err(e),
        };
        async { res?.try_into_row_affected().await }
    }

    /// function the same as [Client::pipeline]
    pub fn pipeline<'a, B, const SYNC_MODE: bool>(
        &mut self,
        pipe: Pipeline<'a, B, SYNC_MODE>,
    ) -> Result<PipelineStream<'a>, Error>
    where
        B: DerefMut<Target = BytesMut>,
    {
        self.try_conn()?
            .client
            .pipeline(pipe)
            .inspect_err(|e| self.try_drop_on_error(e))
    }

    fn try_conn(&mut self) -> Result<&mut PoolClient, Error> {
        match self.conn {
            Some((ref mut conn, _)) => Ok(conn),
            None => Err(DriverDown.into()),
        }
    }

    fn try_drop_on_error(&mut self, e: &Error) {
        if e.is_driver_down() {
            let _ = self.take();
        }
    }

    fn take(&mut self) -> Option<(PoolClient, SemaphorePermit<'p>)> {
        self.conn.take()
    }
}

impl Drop for PoolConnection<'_> {
    fn drop(&mut self) {
        if let Some((conn, _permit)) = self.take() {
            self.pool.conn.lock().unwrap().push_back(conn);
        }
    }
}

struct PoolClient {
    client: Client,
    statements: HashMap<Box<str>, Arc<Statement>>,
}

impl PoolClient {
    fn new(client: Client) -> Self {
        Self {
            client,
            statements: HashMap::new(),
        }
    }
}
