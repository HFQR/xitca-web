mod builder;
mod portal;

use super::{
    client::ClientBorrowMut,
    driver::codec::AsParams,
    error::Error,
    iter::slice_iter,
    prepare::Prepare,
    query::{Query, RowStream},
    statement::{Statement, StatementGuarded},
    types::{ToSql, Type},
};

pub use builder::TransactionBuilder;
pub use portal::Portal;

pub struct Transaction<'a, C>
where
    C: Prepare + Query + ClientBorrowMut,
{
    client: &'a mut C,
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

    fn save_point_query(&self) -> String {
        match self {
            Self::None => "SAVEPOINT".to_string(),
            Self::Auto { depth } => format!("SAVEPOINT sp_{depth}"),
            Self::Custom { name, .. } => format!("SAVEPOINT {name}"),
        }
    }

    fn commit_query(&self) -> String {
        match self {
            Self::None => "COMMIT".to_string(),
            Self::Auto { depth } => format!("RELEASE sp_{depth}"),
            Self::Custom { name, .. } => format!("RELEASE {name}"),
        }
    }

    fn rollback_query(&self) -> String {
        match self {
            Self::None => "ROLLBACK".to_string(),
            Self::Auto { depth } => format!("ROLLBACK TO sp_{depth}"),
            Self::Custom { name, .. } => format!("ROLLBACK TO {name}"),
        }
    }
}

enum State {
    WantRollback,
    Finish,
}

impl<C> Drop for Transaction<'_, C>
where
    C: Prepare + Query + ClientBorrowMut,
{
    fn drop(&mut self) {
        match self.state {
            State::WantRollback => self.do_rollback(),
            State::Finish => {}
        }
    }
}

impl<C> Transaction<'_, C>
where
    C: Prepare + Query + ClientBorrowMut,
{
    pub fn builder() -> TransactionBuilder {
        TransactionBuilder::new()
    }

    /// function the same as [`Client::prepare`]
    ///
    /// [`Client::prepare`]: crate::client::Client::prepare
    pub async fn prepare(&self, query: &str, types: &[Type]) -> Result<StatementGuarded<C>, Error> {
        self.client
            ._prepare(query, types)
            .await
            .map(|stmt| stmt.into_guarded(self.client))
    }

    /// function the same as [`Client::query`]
    ///
    /// [`Client::query`]: crate::client::Client::query
    #[inline]
    pub fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.client._query(stmt, params)
    }

    /// function the same as [`Client::query_raw`]
    ///
    /// [`Client::query_raw`]: crate::client::Client::query_raw
    #[inline]
    pub fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: AsParams,
    {
        self.client._query_raw(stmt, params)
    }

    /// Binds a statement to a set of parameters, creating a [`Portal`] which can be incrementally queried.
    ///
    /// Portals only last for the duration of the transaction in which they are created, and can only be used on the
    /// connection that created them.
    pub async fn bind<'p>(
        &'p self,
        statement: &'p Statement,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Portal<'p, C>, Error> {
        self.bind_raw(statement, slice_iter(params)).await
    }

    /// A maximally flexible version of [`Transaction::bind`].
    pub async fn bind_raw<'p, I>(&'p self, statement: &'p Statement, params: I) -> Result<Portal<'p, C>, Error>
    where
        I: AsParams,
    {
        Portal::new(self.client, statement, params).await
    }

    /// Like [`Client::transaction`], but creates a nested transaction via a savepoint.
    ///     
    /// [`Client::transaction`]: crate::client::Client::transaction
    pub async fn transaction(&mut self) -> Result<Transaction<C>, Error> {
        self._save_point(None).await
    }

    /// Like [`Client::transaction`], but creates a nested transaction via a savepoint with the specified name.
    ///
    /// [`Client::transaction`]: crate::client::Client::transaction
    pub async fn save_point<I>(&mut self, name: I) -> Result<Transaction<C>, Error>
    where
        I: Into<String>,
    {
        self._save_point(Some(name.into())).await
    }

    /// Consumes the transaction, committing all changes made within it.
    pub async fn commit(mut self) -> Result<(), Error> {
        self.state = State::Finish;
        let query = self.save_point.commit_query();
        self.client._execute_raw::<_, crate::ZeroParam>(&query, []).await?;
        Ok(())
    }

    /// Rolls the transaction back, discarding all changes made within it.
    ///
    /// This is equivalent to [`Transaction`]'s [`Drop`] implementation, but provides any error encountered to the caller.
    pub async fn rollback(mut self) -> Result<(), Error> {
        self.state = State::Finish;
        let query = self.save_point.rollback_query();
        self.client._execute_raw::<_, crate::ZeroParam>(&query, []).await?;
        Ok(())
    }

    fn new(client: &mut C) -> Transaction<C> {
        Transaction {
            client,
            save_point: SavePoint::None,
            state: State::WantRollback,
        }
    }

    async fn _save_point(&mut self, name: Option<String>) -> Result<Transaction<C>, Error> {
        let save_point = self.save_point.nest_save_point(name);
        self.client
            ._execute_raw::<_, crate::ZeroParam>(&save_point.save_point_query(), [])
            .await?;

        Ok(Transaction {
            client: self.client,
            save_point,
            state: State::WantRollback,
        })
    }

    fn do_rollback(&mut self) {
        let query = self.save_point.rollback_query();
        drop(self.client._execute_raw::<_, crate::ZeroParam>(&query, []));
    }
}
