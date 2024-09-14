use core::{cell::Cell, future::Future, ops::DerefMut};

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use tokio::sync::{Semaphore, SemaphorePermit};
use xitca_io::bytes::BytesMut;

use super::{
    client::{Client, ClientBorrowMut},
    config::Config,
    copy::{r#Copy, CopyIn, CopyOut},
    driver::codec::Response,
    error::Error,
    iter::{slice_iter, AsyncLendingIterator},
    pipeline::{Pipeline, PipelineStream},
    query::{AsParams, Query, QuerySimple, RowSimpleStream, RowStream},
    session::Session,
    statement::{CancelStatement, Statement},
    transaction::Transaction,
    types::{ToSql, Type},
    BoxedFuture, Postgres,
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
        let conn = match conn {
            Some(conn) => conn,
            None => self.connect().await?,
        };
        Ok(PoolConnection {
            pool: self,
            conn: Some(conn),
            _permit,
            broken: Cell::new(false),
        })
    }

    #[inline(never)]
    fn connect(&self) -> BoxedFuture<'_, Result<PoolClient, Error>> {
        Box::pin(async move {
            let (client, mut driver) = Postgres::new(self.config.clone()).connect().await?;
            tokio::task::spawn(async move {
                while let Ok(Some(_)) = driver.try_next().await {
                    // TODO: add notify listen callback to Pool
                }
            });
            Ok(PoolClient::new(client))
        })
    }
}

/// a RAII type for connection. it manages the lifetime of connection and it's [Statement] cache.
/// a set of public is exposed to interact with them.
pub struct PoolConnection<'a> {
    pool: &'a Pool,
    conn: Option<PoolClient>,
    _permit: SemaphorePermit<'a>,
    broken: Cell<bool>,
}

impl<'p> PoolConnection<'p> {
    /// function like [`Client::prepare`] with some behavior difference:
    /// statement will be cached for future reuse and the return type is [`Arc<Statement>`] smart pointer
    pub async fn prepare(&mut self, query: &str, types: &[Type]) -> Result<Arc<Statement>, Error> {
        match self.conn().statements.get(query) {
            Some(stmt) => Ok(stmt.clone()),
            None => self._prepare(query, types).await,
        }
    }

    #[inline(never)]
    fn _prepare<'a>(&'a mut self, query: &'a str, types: &'a [Type]) -> BoxedFuture<'a, Result<Arc<Statement>, Error>> {
        Box::pin(async move {
            let stmt = self
                .conn()
                .client
                ._prepare(query, types)
                .await
                .map(Arc::new)
                .inspect_err(|e| self.try_drop_on_error(e))?;
            self.conn_mut().statements.insert(Box::from(query), stmt.clone());
            Ok(stmt)
        })
    }

    /// function the same as [`Client::query`]
    #[inline]
    pub fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self._query(stmt, params)
    }

    /// function the same as [`Client::query_raw`]
    #[inline]
    pub fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: AsParams,
    {
        self._query_raw(stmt, params)
    }

    /// function the same as [`Client::query_simple`]
    #[inline]
    pub fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        self._query_simple(stmt)
    }

    /// function the same as [`Client::execute`]
    pub fn execute(
        &self,
        stmt: &Statement,
        params: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<u64, Error>> + Send {
        // TODO: forward to Query::_execute
        let res = self._send_encode(stmt, slice_iter(params));
        async { res?.try_into_row_affected().await }
    }

    /// function the same as [`Client::execute_raw`]
    pub fn execute_raw<I>(&self, stmt: &Statement, params: I) -> impl Future<Output = Result<u64, Error>> + Send
    where
        I: AsParams,
    {
        // TODO: forward to Query::_execute_raw
        let res = self._send_encode(stmt, params);
        async { res?.try_into_row_affected().await }
    }

    /// function the same as [`Client::execute_simple`]
    #[inline]
    pub fn execute_simple(&self, stmt: &str) -> impl Future<Output = Result<u64, Error>> + Send {
        self._execute_simple(stmt)
    }

    /// function the same as [`Client::transaction`]
    #[inline]
    pub fn transaction(&mut self) -> impl Future<Output = Result<Transaction<Self>, Error>> + Send {
        Transaction::new(self)
    }

    /// function the same as [`Client::pipeline`]
    pub fn pipeline<'a, B, const SYNC_MODE: bool>(
        &self,
        pipe: Pipeline<'a, B, SYNC_MODE>,
    ) -> Result<PipelineStream<'a>, Error>
    where
        B: DerefMut<Target = BytesMut>,
    {
        self.conn()
            .client
            .pipeline(pipe)
            .inspect_err(|e| self.try_drop_on_error(e))
    }

    /// function the same as [`Client::copy_in`]
    #[inline]
    pub fn copy_in(&mut self, stmt: &Statement) -> impl Future<Output = Result<CopyIn<Self>, Error>> + Send {
        CopyIn::new(self, stmt)
    }

    /// function the same as [`Client::copy_out`]
    #[inline]
    pub async fn copy_out(&self, stmt: &Statement) -> Result<CopyOut, Error> {
        CopyOut::new(self, stmt).await
    }

    /// a shortcut to move and take ownership of self.
    /// an important behavior of [PoolConnection] is it supports pipelining. eagerly drop it after usage can
    /// contribute to more queries being pipelined. especially before any `await` point.
    ///
    /// # Examples
    /// ```rust
    /// use xitca_postgres::{pool::Pool, Error};
    ///
    /// async fn example(pool: &Pool) -> Result<(), Error> {
    ///     // get a connection from pool and start a query.
    ///     let mut conn = pool.get().await?;
    ///
    ///     conn.execute_simple("SELECT *").await?;
    ///     
    ///     // connection is kept across await point. making it unusable to other concurrent
    ///     // callers to example function. and no pipelining will happen until it's released.
    ///     let conn = conn;
    ///
    ///     // start another query but this time consume ownership and when res is returned
    ///     // connection is dropped and went back to pool.
    ///     let res = conn.consume().execute_simple("SELECT *");
    ///
    ///     // connection can't be used anymore in this scope but other concurrent callers
    ///     // to example function is able to use it and if they follow the same calling
    ///     // convention pipelining could happen and reduce syscall overhead.
    ///     //
    ///     // let res = conn.consume().execute_simple("SELECT *");
    ///
    ///     // without connection the response can still be collected asynchronously
    ///     res.await?;
    ///
    ///     // therefore a good calling convention for independent queries could be:
    ///     let mut conn = pool.get().await?;
    ///     let res1 = conn.execute_simple("SELECT *");
    ///     let res2 = conn.execute_simple("SELECT *");
    ///     let res3 = conn.consume().execute_simple("SELECT *");
    ///
    ///     // all three queries can be pipelined into a single write syscall. and possibly
    ///     // even more can be pipelined after conn.consume() is called if there are concurrent
    ///     // callers use the same connection.
    ///     
    ///     res1.await?;
    ///     res2.await?;
    ///     res3.await?;
    ///
    ///     // it should be noted that pipelining is an optional crate feature for some potential
    ///     // performance gain.
    ///     // it's totally fine to ignore and use the apis normally with zero thought put into it.
    ///
    ///     Ok(())
    /// }
    /// ```
    #[inline(always)]
    pub fn consume(self) -> Self {
        self
    }

    /// function the same as [`Client::cancel_token`]
    pub fn cancel_token(&self) -> Session {
        self.conn().client.cancel_token()
    }

    fn conn(&self) -> &PoolClient {
        self.conn.as_ref().unwrap()
    }

    fn conn_mut(&mut self) -> &mut PoolClient {
        self.conn.as_mut().unwrap()
    }

    #[cold]
    #[inline(never)]
    fn try_drop_on_error(&self, e: &Error) {
        if e.is_driver_down() {
            self.broken.set(true);
        }
    }
}

impl ClientBorrowMut for PoolConnection<'_> {
    fn _borrow_mut(&mut self) -> &mut Client {
        &mut self.conn_mut().client
    }
}

impl CancelStatement for &mut PoolConnection<'_> {
    fn cancel(&self, stmt: &Statement) {
        (&self.conn().client).cancel(stmt)
    }
}

impl Query for PoolConnection<'_> {
    fn _send_encode<I>(&self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams,
    {
        self.conn()
            .client
            ._send_encode(stmt, params)
            .inspect_err(|e| self.try_drop_on_error(e))
    }
}

impl QuerySimple for PoolConnection<'_> {
    fn _send_encode_simple(&self, stmt: &str) -> Result<Response, Error> {
        self.conn()
            .client
            ._send_encode_simple(stmt)
            .inspect_err(|e| self.try_drop_on_error(e))
    }
}

impl r#Copy for PoolConnection<'_> {
    fn send_one_way<F>(&self, func: F) -> Result<(), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self.conn()
            .client
            .send_one_way(func)
            .inspect_err(|e| self.try_drop_on_error(e))
    }
}

impl Drop for PoolConnection<'_> {
    fn drop(&mut self) {
        if self.broken.get() {
            return;
        }
        self.pool.conn.lock().unwrap().push_back(self.conn.take().unwrap());
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
