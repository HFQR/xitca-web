//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use core::{ops::Deref, sync::atomic::Ordering};

use std::sync::Arc;

use super::{
    column::Column,
    driver::codec::{AsParams, encode::StatementCancel},
    query::Query,
    types::{ToSql, Type},
};

/// a statement guard contains a prepared postgres statement.
/// the guard can be dereferenced or borrowed as [`Statement`] which can be used for query apis.
///
/// the guard would cancel it's statement when dropped. generic C type must be a client type impl
/// [`Query`] trait to instruct the cancellation.
pub struct StatementGuarded<'a, C>
where
    C: Query,
{
    stmt: Option<Statement>,
    cli: &'a C,
}

impl<C> AsRef<Statement> for StatementGuarded<'_, C>
where
    C: Query,
{
    #[inline]
    fn as_ref(&self) -> &Statement {
        self
    }
}

impl<C> Deref for StatementGuarded<'_, C>
where
    C: Query,
{
    type Target = Statement;

    fn deref(&self) -> &Self::Target {
        self.stmt.as_ref().unwrap()
    }
}

impl<C> Drop for StatementGuarded<'_, C>
where
    C: Query,
{
    fn drop(&mut self) {
        if let Some(stmt) = self.stmt.take() {
            let _ = self.cli._send_encode_query(StatementCancel { name: stmt.name() });
        }
    }
}

impl<C> StatementGuarded<'_, C>
where
    C: Query,
{
    /// leak the statement and it will lose automatic management
    /// **DOES NOT** cause memory leak
    pub fn leak(mut self) -> Statement {
        self.stmt.take().unwrap()
    }
}

/// named prepared postgres statement without information of which [`Client`] it belongs to and lifetime
/// cycle management
///
/// this type is used as entry point for other statement types like [`StatementGuarded`] and [`StatementUnnamed`].
/// itself is rarely directly used and main direct usage is for statement caching where owner of it is tasked
/// with manual management of it's association and lifetime
///
/// [`Client`]: crate::client::Client
// Statement must not implement Clone trait. use `Statement::duplicate` if needed.
// StatementGuarded impls Deref trait and with Clone trait it will be possible to copy Statement out of a
// StatementGuarded. This is not a desired behavior and obtaining a Statement from it's guard should only
// be possible with StatementGuarded::leak API.
#[derive(Default)]
pub struct Statement {
    name: Arc<str>,
    params: Arc<[Type]>,
    columns: Arc<[Column]>,
}

impl Statement {
    pub(crate) fn new(name: String, params: Vec<Type>, columns: Vec<Column>) -> Self {
        Self {
            name: name.into(),
            params: params.into(),
            columns: columns.into(),
        }
    }

    // cloning of statement inside library must be carefully utlized to keep cancelation happen properly on drop.
    pub(crate) fn duplicate(&self) -> Self {
        Self {
            name: self.name.clone(),
            params: self.params.clone(),
            columns: self.columns.clone(),
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn columns_owned(&self) -> Arc<[Column]> {
        self.columns.clone()
    }

    /// construct a new named statement.
    /// can be called with [`Execute::execute`] method for making a prepared statement.
    ///
    /// [`Execute::execute`]: crate::execute::Execute::execute
    #[inline]
    pub const fn named<'a>(stmt: &'a str, types: &'a [Type]) -> StatementNamed<'a> {
        StatementNamed { stmt, types }
    }

    #[inline]
    pub const fn unnamed<'a>(stmt: &'a str, types: &'a [Type]) -> StatementNamed<'a> {
        StatementNamed { stmt, types }
    }

    /// bind self to typed value parameters where they are encoded into a valid sql query in binary format
    ///
    /// # Examples
    /// ```
    /// # use xitca_postgres::{types::Type, Client, Error, Execute, Statement};
    /// # async fn bind(cli: Client) -> Result<(), Error> {
    /// // prepare a statement with typed parameters.
    /// let stmt = Statement::named("SELECT * FROM users WHERE id = $1 AND age = $2", &[Type::INT4, Type::INT4])
    ///     .execute(&cli).await?;
    /// // bind statement to typed value parameters and start query
    /// let row_stream = stmt.bind([9527_i32, 18]).query(&cli).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn bind<P>(&self, params: P) -> StatementPreparedQuery<'_, P>
    where
        P: AsParams,
    {
        StatementPreparedQuery { stmt: self, params }
    }

    /// [Statement::bind] for dynamic typed parameters
    ///
    /// # Examples
    /// ```
    /// # fn bind_dyn(statement: xitca_postgres::statement::Statement) {
    /// // bind to a dynamic typed slice where items have it's own concrete type.
    /// let bind = statement.bind_dyn(&[&9527i32, &"nobody"]);
    /// # }
    /// ```
    #[inline]
    pub fn bind_dyn<'p, 't>(
        &self,
        params: &'p [&'t (dyn ToSql + Sync)],
    ) -> StatementPreparedQuery<'_, impl ExactSizeIterator<Item = &'t (dyn ToSql + Sync)> + Clone + 'p> {
        self.bind(params.iter().cloned())
    }

    /// Returns the expected types of the statement's parameters.
    #[inline]
    pub fn params(&self) -> &[Type] {
        &self.params
    }

    /// Returns information about the columns returned when the statement is queried.
    #[inline]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Convert self to a drop guarded statement which would cancel on drop.
    #[inline]
    pub fn into_guarded<C>(self, cli: &C) -> StatementGuarded<'_, C>
    where
        C: Query,
    {
        StatementGuarded { stmt: Some(self), cli }
    }
}

/// a named statement that can be prepared separately
pub struct StatementNamed<'a> {
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
}

impl<'a> StatementNamed<'a> {
    fn name() -> String {
        let id = crate::NEXT_ID.fetch_add(1, Ordering::Relaxed);
        format!("s{id}")
    }

    /// function the same as [`Statement::bind`]
    #[inline]
    pub fn bind<P>(self, params: P) -> StatementQuery<'a, P> {
        StatementQuery {
            stmt: self.stmt,
            types: self.types,
            params,
        }
    }

    /// function the same as [`Statement::bind_dyn`]
    #[inline]
    pub fn bind_dyn<'p, 't>(
        self,
        params: &'p [&'t (dyn ToSql + Sync)],
    ) -> StatementQuery<'a, impl ExactSizeIterator<Item = &'t (dyn ToSql + Sync)> + Clone + 'p> {
        self.bind(params.iter().cloned())
    }
}

pub(crate) struct StatementCreate<'a, 'c, C> {
    pub(crate) name: String,
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
    pub(crate) cli: &'c C,
}

impl<'a, 'c, C> From<(StatementNamed<'a>, &'c C)> for StatementCreate<'a, 'c, C> {
    fn from((stmt, cli): (StatementNamed<'a>, &'c C)) -> Self {
        Self {
            name: StatementNamed::name(),
            stmt: stmt.stmt,
            types: stmt.types,
            cli,
        }
    }
}

pub(crate) struct StatementCreateBlocking<'a, 'c, C> {
    pub(crate) name: String,
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
    pub(crate) cli: &'c C,
}

impl<'a, 'c, C> From<(StatementNamed<'a>, &'c C)> for StatementCreateBlocking<'a, 'c, C> {
    fn from((stmt, cli): (StatementNamed<'a>, &'c C)) -> Self {
        Self {
            name: StatementNamed::name(),
            stmt: stmt.stmt,
            types: stmt.types,
            cli,
        }
    }
}

/// a named and already prepared statement with it's query params
///
/// after [`Execute::query`] by certain excutor it would produce [`RowStream`] as response
///
/// [`Execute::query`]: crate::execute::Execute::query
/// [`RowStream`]: crate::query::RowStream
pub struct StatementPreparedQuery<'a, P> {
    pub(crate) stmt: &'a Statement,
    pub(crate) params: P,
}

impl<'a, P> StatementPreparedQuery<'a, P> {
    #[inline]
    pub fn into_owned(self) -> StatementPreparedQueryOwned<'a, P> {
        StatementPreparedQueryOwned {
            stmt: self.stmt,
            params: self.params,
        }
    }
}

/// owned version of [`StatementPreparedQuery`]
///
/// after [`Execute::query`] by certain excutor it would produce [`RowStreamOwned`] as response
///
/// [`Execute::query`]: crate::execute::Execute::query
/// [`RowStreamOwned`]: crate::query::RowStreamOwned
pub struct StatementPreparedQueryOwned<'a, P> {
    pub(crate) stmt: &'a Statement,
    pub(crate) params: P,
}

/// an unprepared statement with it's query params
///
/// Certain executor can make use of unprepared statement and offer addtional functionality
/// # Examples
/// ```rust
/// # use xitca_postgres::{pool::Pool, types::Type, Execute, Statement};
/// async fn execute_with_pool(pool: &Pool) {
///     // connection pool can execute unprepared statement directly where statement preparing
///     // execution and caching happens internally
///     let rows = Statement::named("SELECT * FROM user WHERE id = $1", &[Type::INT4])
///         .bind([9527])
///         .query(pool)
///         .await;
/// }
/// ```
pub struct StatementQuery<'a, P> {
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
    pub(crate) params: P,
}

impl<'a, P> StatementQuery<'a, P> {
    /// transform self to a single use of statement query with given executor
    ///
    /// See [`StatementNoPrepareQuery`] for explaination
    pub fn into_no_prepared<'c, C>(self, cli: &'c C) -> StatementNoPrepareQuery<'a, 'c, P, C> {
        StatementNoPrepareQuery { bind: self, cli }
    }
}

/// an unprepared statement with it's query params and reference of certain executor
/// given executor is tasked with prepare and query with a single round-trip to database and cancel
/// the prepared statement after query is finished
pub struct StatementNoPrepareQuery<'a, 'c, P, C> {
    pub(crate) bind: StatementQuery<'a, P>,
    pub(crate) cli: &'c C,
}

#[cfg(feature = "compat")]
pub(crate) mod compat {
    use core::ops::Deref;

    use std::sync::Arc;

    use super::{Query, Statement, StatementCancel};

    /// functions the same as [`StatementGuarded`]
    ///
    /// instead of work with a reference this guard offers ownership without named lifetime constraint.
    ///
    /// [`StatementGuarded`]: super::StatementGuarded
    pub struct StatementGuarded<C>
    where
        C: Query,
    {
        stmt: Statement,
        cli: C,
    }

    impl<C> Clone for StatementGuarded<C>
    where
        C: Query + Clone,
    {
        fn clone(&self) -> Self {
            Self {
                stmt: self.stmt.duplicate(),
                cli: self.cli.clone(),
            }
        }
    }

    impl<C> Drop for StatementGuarded<C>
    where
        C: Query,
    {
        fn drop(&mut self) {
            // cancel statement when the last copy is about to be dropped.
            if Arc::strong_count(&self.stmt.name) == 1 {
                debug_assert_eq!(Arc::strong_count(&self.stmt.params), 1);
                debug_assert_eq!(Arc::strong_count(&self.stmt.columns), 1);
                let _ = self.cli._send_encode_query(StatementCancel { name: self.stmt.name() });
            }
        }
    }

    impl<C> Deref for StatementGuarded<C>
    where
        C: Query,
    {
        type Target = Statement;

        fn deref(&self) -> &Self::Target {
            &self.stmt
        }
    }

    impl<C> AsRef<Statement> for StatementGuarded<C>
    where
        C: Query,
    {
        fn as_ref(&self) -> &Statement {
            &self.stmt
        }
    }

    impl<C> StatementGuarded<C>
    where
        C: Query,
    {
        /// construct a new statement guard with raw statement and client
        pub fn new(stmt: Statement, cli: C) -> Self {
            Self { stmt, cli }
        }

        /// obtain client reference from guarded statement
        /// can be helpful in use case where clinet object is not cheaply avaiable
        pub fn client(&self) -> &C {
            &self.cli
        }
    }
}

#[cfg(test)]
mod test {
    use core::future::IntoFuture;

    use crate::{
        Postgres,
        error::{DbError, SqlState},
        execute::Execute,
        iter::AsyncLendingIterator,
        statement::Statement,
    };

    #[tokio::test]
    async fn cancel_statement() {
        let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
            .connect()
            .await
            .unwrap();

        tokio::task::spawn(drv.into_future());

        std::path::Path::new("./samples/test.sql").execute(&cli).await.unwrap();

        let stmt = Statement::named("SELECT id, name FROM foo ORDER BY id", &[])
            .execute(&cli)
            .await
            .unwrap();

        let stmt_raw = stmt.duplicate();

        drop(stmt);

        let mut stream = stmt_raw.query(&cli).await.unwrap();

        let e = stream.try_next().await.err().unwrap();

        let e = e.downcast_ref::<DbError>().unwrap();

        assert_eq!(e.code(), &SqlState::INVALID_SQL_STATEMENT_NAME);
    }
}
