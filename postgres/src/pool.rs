use core::{
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use tokio::sync::{Semaphore, SemaphorePermit};
use xitca_io::{bytes::BytesMut, io::AsyncIo};

use super::{
    BoxedFuture, Postgres,
    client::{Client, ClientBorrowMut},
    config::Config,
    copy::{r#Copy, CopyIn, CopyOut},
    driver::{
        Driver,
        codec::{AsParams, Response, encode::Encode},
        generic::GenericDriver,
    },
    error::Error,
    execute::Execute,
    iter::AsyncLendingIterator,
    prepare::Prepare,
    query::{Query, RowAffected, RowStreamOwned},
    session::Session,
    statement::{Statement, StatementNamed, StatementQuery},
    transaction::{Transaction, TransactionBuilder},
    types::{Oid, Type},
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
    /// return as [`Error`]
    pub async fn get(&self) -> Result<PoolConnection<'_>, Error> {
        let _permit = self.permits.acquire().await.expect("Semaphore must not be closed");
        let conn = self.conn.lock().unwrap().pop_front();
        let conn = match conn {
            Some(conn) if !conn.client.closed() => conn,
            _ => self.connect().await?,
        };
        Ok(PoolConnection {
            pool: self,
            conn: Some(conn),
            _permit,
        })
    }

    #[cold]
    #[inline(never)]
    fn connect(&self) -> BoxedFuture<'_, Result<PoolClient, Error>> {
        Box::pin(async move {
            let (client, driver) = Postgres::new(self.config.clone()).connect().await?;
            match driver {
                Driver::Tcp(drv) => drive(drv),
                Driver::Dynamic(drv) => drive(drv),
                #[cfg(feature = "tls")]
                Driver::Tls(drv) => drive(drv),
                #[cfg(unix)]
                Driver::Unix(drv) => drive(drv),
                #[cfg(all(unix, feature = "tls"))]
                Driver::UnixTls(drv) => drive(drv),
                #[cfg(feature = "quic")]
                Driver::Quic(drv) => drive(drv),
            };
            Ok(PoolClient::new(client))
        })
    }
}

fn drive(mut drv: GenericDriver<impl AsyncIo + Send + 'static>) {
    tokio::task::spawn(async move {
        while drv.try_next().await?.is_some() {
            // TODO: add notify listen callback to Pool
        }
        Ok::<_, Error>(())
    });
}

/// a RAII type for connection. it manages the lifetime of connection and it's [`Statement`] cache.
/// a set of public is exposed to interact with them.
///
/// # Caching
/// PoolConnection contains cache set of [`Statement`] to speed up regular used sql queries. when calling
/// [`Execute::execute`] on a [`StatementNamed`] with &[`PoolConnection`] the pool connection does nothing
/// special and function the same as a regular [`Client`]. In order to utilize the cache caller must execute
/// the named statement with &mut [`PoolConnection`]. With a mutable reference of pool connection it will do
/// local cache look up for statement and hand out one in the type of [`Statement`] if any found. If no
/// copy is found in the cache pool connection will prepare a new statement and insert it into the cache.
/// ## Examples
/// ```
/// # use xitca_postgres::{pool::Pool, Execute, Error, Statement};
/// # async fn cached(pool: &Pool) -> Result<(), Error> {
/// let mut conn = pool.get().await?;
/// // prepare a statement without caching
/// Statement::named("SELECT 1", &[]).execute(&conn).await?;
/// // prepare a statement with caching from conn.
/// Statement::named("SELECT 1", &[]).execute(&mut conn).await?;
/// # Ok(())
/// # }
/// ```
///
/// * When to use caching or not:
/// - query statement repeatedly called intensely can benefit from cache.
/// - query statement with low latency requirement can benefit from upfront cache.
/// - rare query statement can benefit from no caching by reduce resource usage from the server side. For low
///   latency of rare query consider use [`StatementNamed::bind`] as alternative.
pub struct PoolConnection<'a> {
    pool: &'a Pool,
    conn: Option<PoolClient>,
    _permit: SemaphorePermit<'a>,
}

impl PoolConnection<'_> {
    /// function the same as [`Client::transaction`]
    #[inline]
    pub fn transaction(&mut self) -> impl Future<Output = Result<Transaction<&mut Self>, Error>> + Send {
        TransactionBuilder::new().begin(self)
    }

    /// owned version of [`PoolConnection::transaction`]
    #[inline]
    pub fn transaction_owned(self) -> impl Future<Output = Result<Transaction<Self>, Error>> + Send {
        TransactionBuilder::new().begin(self)
    }

    /// function the same as [`Client::copy_in`]
    #[inline]
    pub fn copy_in(&mut self, stmt: &Statement) -> impl Future<Output = Result<CopyIn<'_, Self>, Error>> + Send {
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
    /// use xitca_postgres::{pool::Pool, Error, Execute};
    ///
    /// async fn example(pool: &Pool) -> Result<(), Error> {
    ///     // get a connection from pool and start a query.
    ///     let mut conn = pool.get().await?;
    ///
    ///     "SELECT *".execute(&conn).await?;
    ///     
    ///     // connection is kept across await point. making it unusable to other concurrent
    ///     // callers to example function. and no pipelining will happen until it's released.
    ///     conn = conn;
    ///
    ///     // start another query but this time consume ownership and when res is returned
    ///     // connection is dropped and went back to pool.
    ///     let res = "SELECT *".execute(&conn.consume());
    ///
    ///     // connection can't be used anymore in this scope but other concurrent callers
    ///     // to example function is able to use it and if they follow the same calling
    ///     // convention pipelining could happen and reduce syscall overhead.
    ///
    ///     // let res = "SELECT *".execute(&conn);
    ///
    ///     // without connection the response can still be collected asynchronously
    ///     res.await?;
    ///
    ///     // therefore a good calling convention for independent queries could be:
    ///     let conn = pool.get().await?;
    ///     let res1 = "SELECT *".execute(&conn);
    ///     let res2 = "SELECT *".execute(&conn);
    ///     let res3 = "SELECT *".execute(&conn.consume());
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

    fn insert_cache(&mut self, named: &str, stmt: Statement) -> &Statement {
        self.conn_mut().statements.entry(Box::from(named)).or_insert(stmt)
    }

    fn conn(&self) -> &PoolClient {
        self.conn.as_ref().unwrap()
    }

    fn conn_mut(&mut self) -> &mut PoolClient {
        self.conn.as_mut().unwrap()
    }
}

impl ClientBorrowMut for PoolConnection<'_> {
    #[inline]
    fn _borrow_mut(&mut self) -> &mut Client {
        &mut self.conn_mut().client
    }
}

impl Prepare for PoolConnection<'_> {
    #[inline]
    async fn _get_type(&self, oid: Oid) -> Result<Type, Error> {
        self.conn().client._get_type(oid).await
    }

    #[inline]
    fn _get_type_blocking(&self, oid: Oid) -> Result<Type, Error> {
        self.conn().client._get_type_blocking(oid)
    }
}

impl Query for PoolConnection<'_> {
    #[inline]
    fn _send_encode_query<S>(&self, stmt: S) -> Result<(S::Output, Response), Error>
    where
        S: Encode,
    {
        self.conn().client._send_encode_query(stmt)
    }
}

impl r#Copy for PoolConnection<'_> {
    #[inline]
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
        self.pool.conn.lock().unwrap().push_back(conn);
    }
}

struct PoolClient {
    client: Client,
    statements: HashMap<Box<str>, Statement>,
}

impl PoolClient {
    fn new(client: Client) -> Self {
        Self {
            client,
            statements: HashMap::new(),
        }
    }
}

impl<'c, 's> Execute<&'c mut PoolConnection<'_>> for StatementNamed<'s>
where
    's: 'c,
{
    type ExecuteOutput = StatementCacheFuture<'c>;
    type QueryOutput = Self::ExecuteOutput;

    fn execute(self, cli: &'c mut PoolConnection) -> Self::ExecuteOutput {
        match cli.conn().statements.get(self.stmt) {
            Some(stmt) => StatementCacheFuture::Cached(stmt.duplicate()),
            None => StatementCacheFuture::Prepared(Box::pin(async move {
                let name = self.stmt;
                let stmt = self.execute(&*cli).await?.leak();
                Ok(cli.insert_cache(name, stmt).duplicate())
            })),
        }
    }

    #[inline]
    fn query(self, cli: &'c mut PoolConnection) -> Self::QueryOutput {
        self.execute(cli)
    }
}

#[cfg(not(feature = "nightly"))]
impl<'c, 's, P> Execute<&'c mut PoolConnection<'_>> for StatementQuery<'s, P>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = BoxedFuture<'c, Result<RowAffected, Error>>;
    type QueryOutput = BoxedFuture<'c, Result<RowStreamOwned, Error>>;

    fn execute(self, conn: &'c mut PoolConnection<'_>) -> Self::ExecuteOutput {
        Box::pin(async move {
            let StatementQuery { stmt, types, params } = self;

            let stmt = match conn.conn().statements.get(stmt) {
                Some(stmt) => stmt,
                None => {
                    let prepared_stmt = Statement::named(stmt, types).execute(&conn).await?.leak();
                    conn.insert_cache(stmt, prepared_stmt);
                    conn.conn().statements.get(stmt).unwrap()
                }
            };

            stmt.bind(params).query(conn).await.map(RowAffected::from)
        })
    }

    fn query(self, conn: &'c mut PoolConnection<'_>) -> Self::QueryOutput {
        Box::pin(async move {
            let StatementQuery { stmt, types, params } = self;

            let stmt = match conn.conn().statements.get(stmt) {
                Some(stmt) => stmt,
                None => {
                    let prepared_stmt = Statement::named(stmt, types).execute(&conn).await?.leak();
                    conn.insert_cache(stmt, prepared_stmt);
                    conn.conn().statements.get(stmt).unwrap()
                }
            };

            stmt.bind(params).into_owned().query(conn).await
        })
    }
}

#[cfg(feature = "nightly")]
impl<'c, 's, 'p, P> Execute<&'c mut PoolConnection<'p>> for StatementQuery<'s, P>
where
    P: AsParams + Send + 'c,
    's: 'c,
    'p: 'c,
{
    type ExecuteOutput = impl Future<Output = Result<RowAffected, Error>> + Send + 'c;
    type QueryOutput = impl Future<Output = Result<RowStreamOwned, Error>> + Send + 'c;

    fn execute(self, conn: &'c mut PoolConnection<'p>) -> Self::ExecuteOutput {
        async move {
            let StatementQuery { stmt, types, params } = self;

            let stmt = match conn.conn().statements.get(stmt) {
                Some(stmt) => stmt,
                None => {
                    let prepared_stmt = Statement::named(stmt, types).execute(&conn).await?.leak();
                    conn.insert_cache(stmt, prepared_stmt);
                    conn.conn().statements.get(stmt).unwrap()
                }
            };

            stmt.bind(params).query(conn).await.map(RowAffected::from)
        }
    }

    fn query(self, conn: &'c mut PoolConnection<'p>) -> Self::QueryOutput {
        async move {
            let StatementQuery { stmt, types, params } = self;

            let stmt = match conn.conn().statements.get(stmt) {
                Some(stmt) => stmt,
                None => {
                    let prepared_stmt = Statement::named(stmt, types).execute(&conn).await?.leak();
                    conn.insert_cache(stmt, prepared_stmt);
                    conn.conn().statements.get(stmt).unwrap()
                }
            };

            stmt.bind(params).into_owned().query(conn).await
        }
    }
}

// TODO: unbox returned futures when type alias is allowed in associated type.
#[cfg(not(feature = "nightly"))]
impl<'c, 's, P> Execute<&'c Pool> for StatementQuery<'s, P>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = BoxedFuture<'c, Result<u64, Error>>;
    type QueryOutput = BoxedFuture<'c, Result<RowStreamOwned, Error>>;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        Box::pin(async {
            {
                let mut conn = pool.get().await?;
                self.execute(&mut conn).await?
            }
            // return connection to pool before await on execution future
            .await
        })
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        Box::pin(async {
            let mut conn = pool.get().await?;
            self.query(&mut conn).await
        })
    }
}

#[cfg(feature = "nightly")]
impl<'c, 's, P> Execute<&'c Pool> for StatementQuery<'s, P>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = impl Future<Output = Result<u64, Error>> + Send + 'c;
    type QueryOutput = impl Future<Output = Result<RowStreamOwned, Error>> + Send + 'c;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        async {
            {
                let mut conn = pool.get().await?;
                self.execute(&mut conn).await?
            }
            // return connection to pool before await on execution future
            .await
        }
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        async {
            let mut conn = pool.get().await?;
            self.query(&mut conn).await
        }
    }
}

// TODO: unbox returned futures when type alias is allowed in associated type.
#[cfg(not(feature = "nightly"))]
impl<'c, 's, I, P> Execute<&'c Pool> for I
where
    I: IntoIterator,
    I::IntoIter: Iterator<Item = StatementQuery<'s, P>> + Send + 'c,
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = BoxedFuture<'c, Result<u64, Error>>;
    type QueryOutput = BoxedFuture<'c, Result<Vec<RowStreamOwned>, Error>>;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        Box::pin(execute_iter_with_pool(self.into_iter(), pool))
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        Box::pin(query_iter_with_pool(self.into_iter(), pool))
    }
}

#[cfg(feature = "nightly")]
impl<'c, 's, I, P> Execute<&'c Pool> for I
where
    I: IntoIterator,
    I::IntoIter: Iterator<Item = StatementQuery<'s, P>> + Send + 'c,
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = impl Future<Output = Result<u64, Error>> + Send + 'c;
    type QueryOutput = impl Future<Output = Result<Vec<RowStreamOwned>, Error>> + Send + 'c;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        execute_iter_with_pool(self.into_iter(), pool)
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        query_iter_with_pool(self.into_iter(), pool)
    }
}

async fn execute_iter_with_pool<P>(
    iter: impl Iterator<Item = StatementQuery<'_, P>> + Send,
    pool: &Pool,
) -> Result<u64, Error>
where
    P: AsParams + Send,
{
    let mut res = Vec::with_capacity(iter.size_hint().0);

    {
        let mut conn = pool.get().await?;

        for stmt in iter {
            let fut = stmt.execute(&mut conn).await?;
            res.push(fut);
        }
    }

    let mut num = 0;

    for res in res {
        num += res.await?;
    }

    Ok(num)
}

async fn query_iter_with_pool<P>(
    iter: impl Iterator<Item = StatementQuery<'_, P>> + Send,
    pool: &Pool,
) -> Result<Vec<RowStreamOwned>, Error>
where
    P: AsParams + Send,
{
    let mut res = Vec::with_capacity(iter.size_hint().0);

    let mut conn = pool.get().await?;

    for stmt in iter {
        let stream = stmt.query(&mut conn).await?;
        res.push(stream);
    }

    Ok(res)
}

pub enum StatementCacheFuture<'c> {
    Cached(Statement),
    Prepared(BoxedFuture<'c, Result<Statement, Error>>),
    Done,
}

impl Future for StatementCacheFuture<'_> {
    type Output = Result<Statement, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match mem::replace(this, Self::Done) {
            Self::Cached(stmt) => Poll::Ready(Ok(stmt)),
            Self::Prepared(mut fut) => {
                let res = fut.as_mut().poll(cx);
                if res.is_pending() {
                    drop(mem::replace(this, Self::Prepared(fut)));
                }
                res
            }
            Self::Done => panic!("StatementCacheFuture polled after finish"),
        }
    }
}

#[cfg(test)]
mod test {
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
