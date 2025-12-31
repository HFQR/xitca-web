use core::{
    future::Future,
    mem,
    num::NonZeroUsize,
    ops::Deref,
    pin::Pin,
    task::{Context, Poll},
};

use lru::LruCache;
use tokio::sync::SemaphorePermit;
use xitca_io::bytes::BytesMut;

use crate::{
    BoxedFuture,
    client::{Client, ClientBorrowMut},
    copy::{r#Copy, CopyIn, CopyOut},
    driver::codec::{AsParams, Response, encode::Encode},
    error::Error,
    execute::Execute,
    prepare::Prepare,
    query::{Query, RowAffected, RowStreamOwned},
    session::Session,
    statement::{Statement, StatementNamed, StatementQuery},
    transaction::{Transaction, TransactionBuilder},
    types::{Oid, Type},
};

use super::Pool;

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
    pub(super) pool: &'a Pool,
    pub(super) conn: Option<PoolClient>,
    pub(super) _permit: SemaphorePermit<'a>,
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

    fn insert_cache(&mut self, named: &str, stmt: Statement) {
        let conn = self.conn_mut();
        if let Some((_, stmt)) = conn.statements.push(Box::from(named), CachedStatement { stmt }) {
            drop(stmt.stmt.into_guarded(&conn.client));
        }
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

/// Cached [`Statement`] from [`PoolConnection`]
///
/// Can be used for the same purpose without the ability to cancel actively
/// It's lifetime is managed by [`PoolConnection`]
pub struct CachedStatement {
    stmt: Statement,
}

impl Clone for CachedStatement {
    fn clone(&self) -> Self {
        Self {
            stmt: self.stmt.duplicate(),
        }
    }
}

impl Deref for CachedStatement {
    type Target = Statement;

    fn deref(&self) -> &Self::Target {
        &self.stmt
    }
}

pub(super) struct PoolClient {
    pub(super) client: Client,
    pub(super) statements: LruCache<Box<str>, CachedStatement>,
}

impl PoolClient {
    pub(super) fn new(client: Client, cap: NonZeroUsize) -> Self {
        Self {
            client,
            statements: LruCache::new(cap),
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
        match cli.conn_mut().statements.get(self.stmt) {
            Some(stmt) => StatementCacheFuture::Cached(stmt.clone()),
            None => StatementCacheFuture::Prepared(Box::pin(async move {
                let name = self.stmt;
                let stmt = self.execute(&*cli).await?.leak();
                cli.insert_cache(name, stmt);
                Ok(cli.conn().statements.peek_mru().unwrap().1.clone())
            })),
        }
    }

    #[inline]
    fn query(self, cli: &'c mut PoolConnection) -> Self::QueryOutput {
        self.execute(cli)
    }
}

pub enum StatementCacheFuture<'c> {
    Cached(CachedStatement),
    Prepared(BoxedFuture<'c, Result<CachedStatement, Error>>),
    Done,
}

impl Future for StatementCacheFuture<'_> {
    type Output = Result<CachedStatement, Error>;

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

            if conn.conn_mut().statements.get(stmt).is_none() {
                let prepared_stmt = Statement::named(stmt, types).execute(&conn).await?.leak();
                conn.insert_cache(stmt, prepared_stmt);
            }

            let stmt = conn.conn().statements.peek_mru().unwrap().1;

            stmt.bind(params).query(conn).await.map(RowAffected::from)
        })
    }

    fn query(self, conn: &'c mut PoolConnection<'_>) -> Self::QueryOutput {
        Box::pin(async move {
            let StatementQuery { stmt, types, params } = self;

            if conn.conn_mut().statements.get(stmt).is_none() {
                let prepared_stmt = Statement::named(stmt, types).execute(&conn).await?.leak();
                conn.insert_cache(stmt, prepared_stmt);
            }

            let stmt = conn.conn().statements.peek_mru().unwrap().1;

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

            if conn.conn_mut().statements.get(stmt).is_none() {
                let prepared_stmt = Statement::named(stmt, types).execute(&conn).await?.leak();
                conn.insert_cache(stmt, prepared_stmt);
            }

            let stmt = conn.conn().statements.peek_mru().unwrap().1;

            stmt.bind(params).query(conn).await.map(RowAffected::from)
        }
    }

    fn query(self, conn: &'c mut PoolConnection<'p>) -> Self::QueryOutput {
        async move {
            let StatementQuery { stmt, types, params } = self;

            if conn.conn_mut().statements.get(stmt).is_none() {
                let prepared_stmt = Statement::named(stmt, types).execute(&conn).await?.leak();
                conn.insert_cache(stmt, prepared_stmt);
            }

            let stmt = conn.conn().statements.peek_mru().unwrap().1;

            stmt.bind(params).into_owned().query(conn).await
        }
    }
}
