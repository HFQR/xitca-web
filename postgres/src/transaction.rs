mod builder;
mod portal;

use std::borrow::Cow;

use super::{
    client::ClientBorrowMut,
    driver::codec::{AsParams, Response, encode::Encode},
    error::Error,
    execute::Execute,
    pool::PoolConnection,
    prepare::Prepare,
    query::Query,
    statement::Statement,
    types::{Oid, ToSql, Type},
};

pub use builder::{IsolationLevel, TransactionBuilder};
pub use portal::Portal;

pub struct Transaction<C>
where
    C: Query + ClientBorrowMut,
{
    client: C,
    save_point: SavePoint,
    state: State,
}

enum SavePoint {
    None,
    Auto { depth: u32 },
    Custom { name: String, depth: u32 },
}

impl SavePoint {
    fn nest_save_point(&self, name: Option<String>) -> Self {
        match *self {
            Self::None => match name {
                Some(name) => SavePoint::Custom { name, depth: 1 },
                None => SavePoint::Auto { depth: 1 },
            },
            Self::Auto { depth } | Self::Custom { depth, .. } => match name {
                Some(name) => SavePoint::Custom { name, depth },
                None => SavePoint::Auto { depth: depth + 1 },
            },
        }
    }

    fn save_point_query(&self) -> Cow<'static, str> {
        match self {
            Self::None => Cow::Borrowed("SAVEPOINT"),
            Self::Auto { depth } => Cow::Owned(format!("SAVEPOINT sp_{depth}")),
            Self::Custom { name, .. } => Cow::Owned(format!("SAVEPOINT {name}")),
        }
    }

    fn commit_query(&self) -> Cow<'static, str> {
        match self {
            Self::None => Cow::Borrowed("COMMIT"),
            Self::Auto { depth } => Cow::Owned(format!("RELEASE sp_{depth}")),
            Self::Custom { name, .. } => Cow::Owned(format!("RELEASE {name}")),
        }
    }

    fn rollback_query(&self) -> Cow<'static, str> {
        match self {
            Self::None => Cow::Borrowed("ROLLBACK"),
            Self::Auto { depth } => Cow::Owned(format!("ROLLBACK TO sp_{depth}")),
            Self::Custom { name, .. } => Cow::Owned(format!("ROLLBACK TO {name}")),
        }
    }
}

enum State {
    WantRollback,
    Finish,
}

impl<C> Drop for Transaction<C>
where
    C: Query + ClientBorrowMut,
{
    fn drop(&mut self) {
        match self.state {
            State::WantRollback => self.do_rollback(),
            State::Finish => {}
        }
    }
}

impl<C> Transaction<C>
where
    C: Query + ClientBorrowMut,
{
    /// Binds a statement to a set of parameters, creating a [`Portal`] which can be incrementally queried.
    ///
    /// Portals only last for the duration of the transaction in which they are created, and can only be used on the
    /// connection that created them.
    pub async fn bind<'p>(
        &'p self,
        statement: &'p Statement,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Portal<'p, C>, Error> {
        self.bind_raw(statement, params.iter().cloned()).await
    }

    /// A maximally flexible version of [`Transaction::bind`].
    pub async fn bind_raw<'p, I>(&'p self, statement: &'p Statement, params: I) -> Result<Portal<'p, C>, Error>
    where
        I: AsParams,
    {
        Portal::new(&self.client, statement, params).await
    }

    /// Like [`Client::transaction`], but creates a nested transaction via a savepoint.
    ///     
    /// [`Client::transaction`]: crate::client::Client::transaction
    pub async fn transaction(&mut self) -> Result<Transaction<&mut C>, Error> {
        self._save_point(None).await
    }

    /// Like [`Client::transaction`], but creates a nested transaction via a savepoint with the specified name.
    ///
    /// [`Client::transaction`]: crate::client::Client::transaction
    pub async fn save_point<I>(&mut self, name: I) -> Result<Transaction<&mut C>, Error>
    where
        I: Into<String>,
    {
        self._save_point(Some(name.into())).await
    }

    /// Consumes the transaction, committing all changes made within it.
    pub async fn commit(mut self) -> Result<(), Error> {
        self.state = State::Finish;
        self.save_point.commit_query().execute(&self).await?;
        Ok(())
    }

    /// Rolls the transaction back, discarding all changes made within it.
    ///
    /// This is equivalent to [`Transaction`]'s [`Drop`] implementation, but provides any error encountered to the caller.
    pub async fn rollback(mut self) -> Result<(), Error> {
        self.state = State::Finish;
        self.save_point.rollback_query().execute(&self).await?;
        Ok(())
    }

    fn new(client: C) -> Transaction<C> {
        Transaction {
            client,
            save_point: SavePoint::None,
            state: State::WantRollback,
        }
    }

    async fn _save_point(&mut self, name: Option<String>) -> Result<Transaction<&mut C>, Error> {
        let save_point = self.save_point.nest_save_point(name);
        save_point.save_point_query().execute(&self).await?;

        Ok(Transaction {
            client: &mut self.client,
            save_point,
            state: State::WantRollback,
        })
    }

    fn do_rollback(&mut self) {
        drop(self.save_point.rollback_query().execute(&self));
    }
}

impl<C> Prepare for Transaction<C>
where
    C: Prepare + ClientBorrowMut,
{
    #[inline]
    async fn _get_type(&self, oid: Oid) -> Result<Type, Error> {
        self.client._get_type(oid).await
    }

    #[inline]
    fn _get_type_blocking(&self, oid: Oid) -> Result<Type, Error> {
        self.client._get_type_blocking(oid)
    }
}

impl<C> Query for Transaction<C>
where
    C: Query + ClientBorrowMut,
{
    #[inline]
    fn _send_encode_query<S>(&self, stmt: S) -> Result<(S::Output, Response), Error>
    where
        S: Encode,
    {
        self.client._send_encode_query(stmt)
    }
}

// special treatment for pool connection for it's internal caching logic that are not accessible through Query trait

impl<'c, 'p, Q, EO, QO> Execute<&'c mut Transaction<PoolConnection<'p>>> for Q
where
    Q: Execute<&'c mut PoolConnection<'p>, ExecuteOutput = EO, QueryOutput = QO>,
{
    type ExecuteOutput = EO;
    type QueryOutput = QO;

    #[inline]
    fn execute(self, cli: &'c mut Transaction<PoolConnection<'p>>) -> Self::ExecuteOutput {
        Q::execute(self, &mut cli.client)
    }

    #[inline]
    fn query(self, cli: &'c mut Transaction<PoolConnection<'p>>) -> Self::QueryOutput {
        Q::query(self, &mut cli.client)
    }
}

impl<'c, 'p, Q, EO, QO> Execute<&'c mut Transaction<&mut PoolConnection<'p>>> for Q
where
    Q: Execute<&'c mut PoolConnection<'p>, ExecuteOutput = EO, QueryOutput = QO>,
{
    type ExecuteOutput = EO;
    type QueryOutput = QO;

    #[inline]
    fn execute(self, cli: &'c mut Transaction<&mut PoolConnection<'p>>) -> Self::ExecuteOutput {
        Q::execute(self, &mut cli.client)
    }

    #[inline]
    fn query(self, cli: &'c mut Transaction<&mut PoolConnection<'p>>) -> Self::QueryOutput {
        Q::query(self, &mut cli.client)
    }
}
