//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use core::{ops::Deref, sync::atomic::Ordering};

use super::{
    column::Column,
    driver::codec::{encode::StatementCancel, AsParams},
    prepare::Prepare,
    types::{ToSql, Type},
};

/// a statement guard contains a prepared postgres statement.
/// the guard can be dereferenced or borrowed as [`Statement`] which can be used for query apis.
///
/// the guard would cancel it's statement when dropped. generic C type must be a client type impl
/// [`Prepare`] trait to instruct the cancellation.
pub struct StatementGuarded<'a, C>
where
    C: Prepare,
{
    stmt: Option<Statement>,
    cli: &'a C,
}

impl<C> AsRef<Statement> for StatementGuarded<'_, C>
where
    C: Prepare,
{
    #[inline]
    fn as_ref(&self) -> &Statement {
        self
    }
}

impl<C> Deref for StatementGuarded<'_, C>
where
    C: Prepare,
{
    type Target = Statement;

    fn deref(&self) -> &Self::Target {
        self.stmt.as_ref().unwrap()
    }
}

impl<C> Drop for StatementGuarded<'_, C>
where
    C: Prepare,
{
    fn drop(&mut self) {
        if let Some(stmt) = self.stmt.take() {
            let _ = self.cli._send_encode_query(StatementCancel { name: stmt.name() });
        }
    }
}

impl<C> StatementGuarded<'_, C>
where
    C: Prepare,
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
    name: Box<str>,
    params: Box<[Type]>,
    columns: Box<[Column]>,
}

impl Statement {
    pub(crate) fn new(name: String, params: Vec<Type>, columns: Vec<Column>) -> Self {
        Self {
            name: name.into_boxed_str(),
            params: params.into_boxed_slice(),
            columns: columns.into_boxed_slice(),
        }
    }

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

    /// construct a new named statement.
    /// must be called with [`Execute::execute`] method for making a prepared statement.
    ///
    /// [`Execute::execute`]: crate::execute::Execute::execute
    #[inline]
    pub fn named<'a>(stmt: &'a str, types: &'a [Type]) -> StatementNamed<'a> {
        let id = crate::NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("s{id}");
        StatementNamed { name, stmt, types }
    }

    /// construct a new unnamed statement.
    /// unnamed statement can bind to it's parameter values without being prepared by database.
    #[inline]
    pub fn unnamed<'a>(stmt: &'a str, types: &'a [Type]) -> StatementUnnamed<'a> {
        StatementUnnamed { stmt, types }
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
    /// let row_stream = stmt.bind([9527_i32, 18]).query(&cli)?;
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn bind<P>(&self, params: P) -> StatementQuery<'_, P>
    where
        P: AsParams,
    {
        StatementQuery { stmt: self, params }
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
    ) -> StatementQuery<'_, impl ExactSizeIterator<Item = &'t (dyn ToSql + Sync)> + 'p> {
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
    pub fn into_guarded<C>(self, cli: &C) -> StatementGuarded<C>
    where
        C: Prepare,
    {
        StatementGuarded { stmt: Some(self), cli }
    }
}

pub struct StatementNamed<'a> {
    pub(crate) name: String,
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
}

pub(crate) struct StatementCreate<'a, C> {
    pub(crate) name: String,
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
    pub(crate) cli: &'a C,
}

pub(crate) struct StatementCreateBlocking<'a, C> {
    pub(crate) name: String,
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
    pub(crate) cli: &'a C,
}

/// an unnamed statement that don't need to be prepared separately
/// it's bundled together with database query it associated that can be processed
/// with at least one-round-trip to database
pub struct StatementUnnamed<'a> {
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
}

impl<'a> StatementUnnamed<'a> {
    /// function the same as [`Statement::bind`]
    #[inline]
    pub fn bind<P>(self, params: P) -> StatementUnnamedBind<'a, P> {
        StatementUnnamedBind {
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
    ) -> StatementUnnamedBind<'a, impl ExactSizeIterator<Item = &'t (dyn ToSql + Sync)> + 'p> {
        self.bind(params.iter().cloned())
    }
}

/// a named statement with it's query params
pub struct StatementQuery<'a, P> {
    pub(crate) stmt: &'a Statement,
    pub(crate) params: P,
}

/// an unnamed statement with it's query param binding
pub struct StatementUnnamedBind<'a, P> {
    stmt: &'a str,
    types: &'a [Type],
    params: P,
}

pub(crate) struct StatementUnnamedQuery<'a, P, C> {
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
    pub(crate) params: P,
    pub(crate) cli: &'a C,
}

impl<'a, P, C> From<(StatementUnnamedBind<'a, P>, &'a C)> for StatementUnnamedQuery<'a, P, C> {
    fn from((bind, cli): (StatementUnnamedBind<'a, P>, &'a C)) -> Self {
        Self {
            stmt: bind.stmt,
            types: bind.types,
            params: bind.params,
            cli,
        }
    }
}

#[cfg(feature = "compat")]
pub(crate) mod compat {
    use core::ops::Deref;

    use std::sync::Arc;

    use super::{Prepare, Statement, StatementCancel};

    /// functions the same as [`StatementGuarded`]
    ///
    /// instead of work with a reference this guard offers ownership without named lifetime constraint.
    ///
    /// [`StatementGuarded`]: super::StatementGuarded
    #[derive(Clone)]
    pub struct StatementGuarded<C>
    where
        C: Prepare,
    {
        inner: Arc<_StatementGuarded<C>>,
    }

    struct _StatementGuarded<C>
    where
        C: Prepare,
    {
        stmt: Statement,
        cli: C,
    }

    impl<C> Drop for _StatementGuarded<C>
    where
        C: Prepare,
    {
        fn drop(&mut self) {
            let _ = self.cli._send_encode_query(StatementCancel { name: self.stmt.name() });
        }
    }

    impl<C> Deref for StatementGuarded<C>
    where
        C: Prepare,
    {
        type Target = Statement;

        fn deref(&self) -> &Self::Target {
            &self.inner.stmt
        }
    }

    impl<C> AsRef<Statement> for StatementGuarded<C>
    where
        C: Prepare,
    {
        fn as_ref(&self) -> &Statement {
            &self.inner.stmt
        }
    }

    impl<C> StatementGuarded<C>
    where
        C: Prepare,
    {
        /// construct a new statement guard with raw statement and client
        pub fn new(stmt: Statement, cli: C) -> Self {
            Self {
                inner: Arc::new(_StatementGuarded { stmt, cli }),
            }
        }
    }
}

#[cfg(test)]
mod test {
    use core::future::IntoFuture;

    use crate::{
        error::{DbError, SqlState},
        execute::Execute,
        iter::AsyncLendingIterator,
        statement::Statement,
        Postgres,
    };

    #[tokio::test]
    async fn cancel_statement() {
        let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
            .connect()
            .await
            .unwrap();

        tokio::task::spawn(drv.into_future());

        "CREATE TEMPORARY TABLE foo (id SERIAL, name TEXT); INSERT INTO foo (name) VALUES ('alice'), ('bob'), ('charlie');"
            .execute(&cli)
            .await
            .unwrap();

        let stmt = Statement::named("SELECT id, name FROM foo ORDER BY id", &[])
            .execute(&cli)
            .await
            .unwrap();

        let stmt_raw = stmt.duplicate();

        drop(stmt);

        let mut stream = stmt_raw.query(&cli).unwrap();

        let e = stream.try_next().await.err().unwrap();

        let e = e.downcast_ref::<DbError>().unwrap();

        assert_eq!(e.code(), &SqlState::INVALID_SQL_STATEMENT_NAME);
    }
}
