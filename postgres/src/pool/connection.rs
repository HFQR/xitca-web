use core::{future::Future, num::NonZeroUsize, ops::Deref};

use lru::LruCache;
use xitca_io::bytes::BytesMut;

use crate::{
    client::{Client, ClientBorrow, ClientBorrowMut},
    copy::{r#Copy, CopyIn, CopyOut},
    driver::codec::AsParams,
    error::Error,
    execute::Execute,
    query::{RowAffected, RowStreamOwned},
    session::Session,
    statement::{Statement, StatementNamed, StatementQuery},
    transaction::{Transaction, TransactionBuilder},
};

use super::PermitLike;

/// a RAII type for connection. it manages the lifetime of connection and it's [`CachedStatement`] cache.
/// a set of public is exposed to interact with them.
///
/// # Caching
/// PoolConnection contains cache set of [`CachedStatement`] to speed up regular used sql queries. when calling
/// [`Execute::execute`] on a [`StatementNamed`] with &[`PoolConnection`] the pool connection does nothing
/// special and function the same as a regular [`Client`]. In order to utilize the cache caller must execute
/// the named statement with &mut [`PoolConnection`]. With a mutable reference of pool connection it will do
/// local cache look up for statement and hand out one in the type of [`CachedStatement`] if any found. If no
/// copy is found in the cache pool connection will prepare a new statement and insert it into the cache.
///
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
///
/// # Pipeline
/// a request is encoded when its future is first polled. requests encoded before the driver performs its
/// next write are batched into a single write syscall. concurrent polling is therefore what decides
/// pipelining: awaiting queries one after another costs a round trip each because every future is polled
/// only after the one before it resolved.
///
/// ## Examples
/// ```
/// # use xitca_postgres::{pool::Pool, Execute, Error};
/// # async fn pipeline(pool: &Pool) -> Result<(), Error> {
/// let conn = pool.get().await?;
///
/// // polled together so all three are encoded before the driver writes. they go out in one syscall.
/// let (res1, res2, res3) = tokio::join!(
///     "SELECT 1".execute(&conn),
///     "SELECT 2".execute(&conn),
///     "SELECT 3".execute(&conn)
/// );
///
/// res1?;
/// res2?;
/// res3?;
///
/// // correct but every query costs a round trip.
/// "SELECT 1".execute(&conn).await?;
/// "SELECT 2".execute(&conn).await?;
/// # Ok(())
/// # }
/// ```
///
/// pipelining is an optional performance gain. it's fine to ignore it and use the apis normally with zero
/// thought put into it.
pub struct GenericPoolConnection<P: PermitLike> {
    pub(super) conn: Option<PoolClient>,
    pub(super) pool_ref: P,
}

impl<'a, P> GenericPoolConnection<P>
where
    P: PermitLike + 'a,
{
    /// function the same as [`Client::transaction`]
    #[inline]
    pub fn transaction(&mut self) -> impl Future<Output = Result<Transaction<'_, Self>, Error>> + Send {
        TransactionBuilder::new().begin(self)
    }

    /// owned version of [`PoolConnection::transaction`]
    #[inline]
    pub fn transaction_owned(self) -> impl Future<Output = Result<Transaction<'a, Self>, Error>> + Send {
        TransactionBuilder::new().begin_owned(self)
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

    /// function the same as [`Client::cancel_token`]
    pub fn cancel_token(&self) -> Session {
        self.conn().client.cancel_token()
    }

    fn insert_cache<'c>(cache: &'c mut Cache, cli: &Client, named: &str, stmt: Statement) -> &'c CachedStatement {
        if let Some((_, stmt)) = cache.push(Box::from(named), CachedStatement { stmt }) {
            drop(stmt.stmt.into_guarded(&cli));
        }
        cache.peek_mru().unwrap().1
    }

    fn conn(&self) -> &PoolClient {
        self.conn.as_ref().unwrap()
    }

    fn conn_mut(&mut self) -> &mut PoolClient {
        self.conn.as_mut().unwrap()
    }
}

impl<P> ClientBorrow for GenericPoolConnection<P>
where
    P: PermitLike,
{
    #[inline]
    fn borrow_cli_ref(&self) -> &Client {
        &self.conn().client
    }
}

impl<P> ClientBorrowMut for GenericPoolConnection<P>
where
    P: PermitLike,
{
    #[inline]
    fn borrow_cli_mut(&mut self) -> &mut Client {
        &mut self.conn_mut().client
    }
}

impl<P> r#Copy for GenericPoolConnection<P>
where
    P: PermitLike,
{
    #[inline]
    fn send_one_way<F>(&self, func: F) -> Result<(), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self.conn().client.send_one_way(func)
    }
}

impl<P> Drop for GenericPoolConnection<P>
where
    P: PermitLike,
{
    fn drop(&mut self) {
        let conn = self.conn.take().unwrap();
        self.pool_ref.put_back(conn);
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

pub struct PoolClient {
    client: Client,
    cache: Cache,
}

impl PoolClient {
    pub(super) fn closed(&self) -> bool {
        self.client.closed()
    }

    pub(super) fn has_capacity(&self) -> bool {
        self.client.tx.has_capacity()
    }
}

type Cache = LruCache<Box<str>, CachedStatement>;

impl PoolClient {
    pub(super) fn new(client: Client, cap: NonZeroUsize) -> Self {
        Self {
            client,
            cache: LruCache::new(cap),
        }
    }
}

impl<'c, P, E> Execute<&'c GenericPoolConnection<P>> for E
where
    P: PermitLike,
    E: Execute<&'c Client>,
{
    type ExecuteOutput = E::ExecuteOutput;
    type QueryOutput = E::QueryOutput;

    #[inline]
    fn execute(
        self,
        cli: &'c GenericPoolConnection<P>,
    ) -> impl Future<Output = Result<Self::ExecuteOutput, Error>> + Send {
        E::execute(self, cli.borrow_cli_ref())
    }

    #[inline]
    fn query(self, cli: &'c GenericPoolConnection<P>) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        E::query(self, cli.borrow_cli_ref())
    }
}

impl<'c, 's, P> Execute<&'c mut GenericPoolConnection<P>> for StatementNamed<'s>
where
    's: 'c,
    P: PermitLike,
{
    type ExecuteOutput = CachedStatement;
    type QueryOutput = Self::ExecuteOutput;

    async fn execute(self, cli: &'c mut GenericPoolConnection<P>) -> Result<Self::ExecuteOutput, Error> {
        // early return keeps the cache borrow out of the miss branch.
        if let Some(stmt) = cli.conn_mut().cache.get(self.stmt) {
            return Ok(stmt.clone());
        }

        let name = self.stmt;
        let stmt = self.execute(&cli.conn_mut().client).await?.leak();
        let conn = cli.conn_mut();
        Ok(GenericPoolConnection::<P>::insert_cache(&mut conn.cache, &conn.client, name, stmt).clone())
    }

    #[inline]
    fn query(
        self,
        cli: &'c mut GenericPoolConnection<P>,
    ) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        self.execute(cli)
    }
}

impl<'c, 's, PP, P> Execute<&'c mut GenericPoolConnection<PP>> for StatementQuery<'s, P>
where
    P: AsParams + Send + 'c,
    PP: PermitLike,
    's: 'c,
{
    type ExecuteOutput = RowAffected;
    type QueryOutput = RowStreamOwned;

    async fn execute(self, conn: &'c mut GenericPoolConnection<PP>) -> Result<Self::ExecuteOutput, Error> {
        let StatementQuery { stmt, types, params } = self;

        let conn = conn.conn_mut();

        let stmt = match conn.cache.get(stmt) {
            Some(stmt) => stmt,
            None => {
                let prepared_stmt = Statement::named(stmt, types).execute(&conn.client).await?.leak();
                GenericPoolConnection::<PP>::insert_cache(&mut conn.cache, &conn.client, stmt, prepared_stmt)
            }
        };

        stmt.bind(params).query(&conn.client).await.map(RowAffected::from)
    }

    async fn query(self, conn: &'c mut GenericPoolConnection<PP>) -> Result<Self::QueryOutput, Error> {
        let StatementQuery { stmt, types, params } = self;

        let conn = conn.conn_mut();

        let stmt = match conn.cache.get(stmt) {
            Some(stmt) => stmt,
            None => {
                let prepared_stmt = Statement::named(stmt, types).execute(&conn.client).await?.leak();
                GenericPoolConnection::<PP>::insert_cache(&mut conn.cache, &conn.client, stmt, prepared_stmt)
            }
        };

        stmt.bind(params).into_owned().query(&conn.client).await
    }
}
