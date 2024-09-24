use core::{future::Future, ops::DerefMut};

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
    driver::codec::{Encode, IntoStream, Response},
    error::Error,
    iter::AsyncLendingIterator,
    pipeline::{Pipeline, PipelineStream},
    prepare::Prepare,
    query::{ExecuteFuture, Query, RowStreamGuarded},
    session::Session,
    statement::{Statement, StatementGuarded},
    transaction::Transaction,
    types::{Oid, ToSql, Type},
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
}

impl<'p> PoolConnection<'p> {
    /// function the same as [`Client::prepare`]
    pub async fn prepare(&self, query: &str, types: &[Type]) -> Result<StatementGuarded<Self>, Error> {
        self._prepare(query, types).await.map(|stmt| stmt.into_guarded(self))
    }

    /// function like [`Client::prepare`] with some behavior difference:
    /// - statement will be cached for future reuse.
    /// - statement type is [`Arc<Statement>`] smart pointer
    ///
    /// * When to use cached prepare or plain prepare:
    /// - query repeatedly called intensely can benefit from cached statement.
    /// - query with low latency requirement can benefit from upfront cached statement.
    /// - rare query can use plain prepare to reduce resource usage from the server side.
    pub async fn prepare_cache(&mut self, query: &str, types: &[Type]) -> Result<Arc<Statement>, Error> {
        match self.conn().statements.get(query) {
            Some(stmt) => Ok(stmt.clone()),
            None => self.prepare_slow(query, types).await,
        }
    }

    #[inline(never)]
    fn prepare_slow<'a>(
        &'a mut self,
        query: &'a str,
        types: &'a [Type],
    ) -> BoxedFuture<'a, Result<Arc<Statement>, Error>> {
        Box::pin(async move {
            let stmt = self._prepare(query, types).await.map(Arc::new)?;
            self.conn_mut().statements.insert(Box::from(query), stmt.clone());
            Ok(stmt)
        })
    }

    /// function the same as [`Client::query`]
    #[inline]
    pub fn query<'a, S>(&self, stmt: S) -> Result<<S::Output<'a> as IntoStream>::RowStream<'a>, Error>
    where
        S: Encode + 'a,
    {
        self._query(stmt)
    }

    /// function the same as [`Client::execute`]
    #[inline]
    pub fn execute<S>(&self, stmt: S) -> ExecuteFuture
    where
        S: Encode,
    {
        self._execute(stmt)
    }

    /// function the same as [`Client::query_unnamed`]
    #[inline]
    pub fn query_unnamed<'a>(
        &'a self,
        stmt: &'a str,
        types: &'a [Type],
        params: &'a [&(dyn ToSql + Sync)],
    ) -> Result<RowStreamGuarded<'a, Self>, Error> {
        self.query((Statement::unnamed(self, stmt, types), params.iter().cloned()))
    }

    /// function the same as [`Client::transaction`]
    #[inline]
    pub fn transaction(&mut self) -> impl Future<Output = Result<Transaction<Self>, Error>> + Send {
        Transaction::<Self>::builder().begin(self)
    }

    /// function the same as [`Client::pipeline`]
    pub fn pipeline<'a, B, const SYNC_MODE: bool>(
        &self,
        pipe: Pipeline<'a, B, SYNC_MODE>,
    ) -> Result<PipelineStream<'a>, Error>
    where
        B: DerefMut<Target = BytesMut>,
    {
        self.conn().client.pipeline(pipe)
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
    ///     conn.execute("SELECT *").await?;
    ///     
    ///     // connection is kept across await point. making it unusable to other concurrent
    ///     // callers to example function. and no pipelining will happen until it's released.
    ///     let conn = conn;
    ///
    ///     // start another query but this time consume ownership and when res is returned
    ///     // connection is dropped and went back to pool.
    ///     let res = conn.consume().execute("SELECT *");
    ///
    ///     // connection can't be used anymore in this scope but other concurrent callers
    ///     // to example function is able to use it and if they follow the same calling
    ///     // convention pipelining could happen and reduce syscall overhead.
    ///     //
    ///     // let res = conn.consume().execute("SELECT *");
    ///
    ///     // without connection the response can still be collected asynchronously
    ///     res.await?;
    ///
    ///     // therefore a good calling convention for independent queries could be:
    ///     let mut conn = pool.get().await?;
    ///     let res1 = conn.execute("SELECT *");
    ///     let res2 = conn.execute("SELECT *");
    ///     let res3 = conn.consume().execute("SELECT *");
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
}

impl ClientBorrowMut for PoolConnection<'_> {
    fn _borrow_mut(&mut self) -> &mut Client {
        &mut self.conn_mut().client
    }
}

impl Prepare for PoolConnection<'_> {
    fn _get_type(&self, oid: Oid) -> BoxedFuture<'_, Result<Type, Error>> {
        self.conn().client._get_type(oid)
    }
}

impl Query for PoolConnection<'_> {
    fn _send_encode_query<'a, S>(&self, stmt: S) -> Result<(S::Output<'a>, Response), Error>
    where
        S: Encode + 'a,
    {
        self.conn().client._send_encode_query(stmt)
    }
}

impl r#Copy for PoolConnection<'_> {
    fn send_one_way<F>(&self, func: F) -> Result<(), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self.conn().client.send_one_way(func)
    }
}

impl Drop for PoolConnection<'_> {
    fn drop(&mut self) {
        let conn = self.conn.take().unwrap();
        if conn.client.closed() {
            return;
        }
        self.pool.conn.lock().unwrap().push_back(conn);
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
