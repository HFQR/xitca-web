mod builder;
mod portal;

use std::borrow::Cow;

use super::{
    client::{Client, ClientBorrow, ClientBorrowMut},
    driver::codec::AsParams,
    error::Error,
    execute::Execute,
    pool::PoolConnection,
    statement::Statement,
    types::ToSql,
};

pub use builder::{IsolationLevel, TransactionBuilder};
pub use portal::Portal;

struct SavePoint {
    name: Name,
    state: State,
}

enum Name {
    Root,
    Nest { name: Option<String>, depth: u32 },
}

impl Default for SavePoint {
    fn default() -> Self {
        Self {
            name: Name::Root,
            state: State::WantRollback,
        }
    }
}

impl SavePoint {
    // rollback runs in Drop trait impl. the execution part has to run eargerly
    fn rollback(&mut self, cli: impl ClientBorrowMut) -> impl Future<Output = Result<(), Error>> + Send {
        self.state = State::Finish;

        let fut = match self.name {
            Name::Root => Cow::Borrowed("ROLLBACK"),
            Name::Nest { ref name, depth } => match name {
                None => Cow::Owned(format!("ROLLBACK TO sp_{depth}")),
                Some(name) => Cow::Owned(format!("ROLLBACK TO {name}")),
            },
        }
        .execute(cli.borrow_cli_ref());

        async { fut.await.map(|_| ()) }
    }

    async fn commit(&mut self, cli: impl ClientBorrowMut) -> Result<(), Error> {
        self.state = State::Finish;

        match self.name {
            Name::Root => Cow::Borrowed("COMMIT"),
            Name::Nest { ref name, depth } => match name {
                None => Cow::Owned(format!("RELEASE sp_{depth}")),
                Some(name) => Cow::Owned(format!("RELEASE {name}")),
            },
        }
        .execute(cli.borrow_cli_ref())
        .await
        .map(|_| ())
    }

    async fn nest_save_point(&self, cli: impl ClientBorrowMut, name: Option<String>) -> Result<Self, Error> {
        let (query, name) = match self.name {
            Name::Root => {
                let query = match name {
                    Some(ref name) => Cow::Owned(format!("SAVEPOINT {name}")),
                    None => Cow::Borrowed("SAVEPOINT sp_1"),
                };
                (query, Name::Nest { name, depth: 1 })
            }
            Name::Nest { depth, .. } => match name {
                Some(name) => (
                    Cow::Owned(format!("SAVEPOINT {name}")),
                    Name::Nest {
                        name: Some(name),
                        depth,
                    },
                ),
                None => {
                    let depth = depth + 1;
                    (
                        Cow::Owned(format!("SAVEPOINT sp_{depth}")),
                        Name::Nest { name: None, depth },
                    )
                }
            },
        };

        query.execute(cli.borrow_cli_ref()).await.map(|_| SavePoint {
            name,
            state: State::WantRollback,
        })
    }

    fn on_drop(&mut self, cli: impl ClientBorrowMut) {
        if matches!(self.state, State::WantRollback) {
            drop(self.rollback(cli));
        }
    }
}

enum State {
    WantRollback,
    Finish,
}

pub struct Transaction<'a, C>
where
    C: ClientBorrowMut,
{
    client: &'a mut C,
    save_point: SavePoint,
}

impl<C> Drop for Transaction<'_, C>
where
    C: ClientBorrowMut,
{
    fn drop(&mut self) {
        self.save_point.on_drop(&mut self.client);
    }
}

impl<C> Transaction<'_, C>
where
    C: ClientBorrowMut,
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
        Portal::new(&*self.client, statement, params).await
    }

    /// Like [`Client::transaction`], but creates a nested transaction via a savepoint.
    ///     
    /// [`Client::transaction`]: crate::client::Client::transaction
    pub async fn transaction(&mut self) -> Result<Transaction<'_, C>, Error> {
        self._save_point(None).await
    }

    /// Like [`Client::transaction`], but creates a nested transaction via a savepoint with the specified name.
    ///
    /// [`Client::transaction`]: crate::client::Client::transaction
    pub async fn save_point<I>(&mut self, name: I) -> Result<Transaction<'_, C>, Error>
    where
        I: Into<String>,
    {
        self._save_point(Some(name.into())).await
    }

    /// Consumes the transaction, committing all changes made within it.
    pub async fn commit(mut self) -> Result<(), Error> {
        self.save_point.commit(&mut self.client).await
    }

    /// Rolls the transaction back, discarding all changes made within it.
    ///
    /// This is equivalent to [`Transaction`]'s [`Drop`] implementation, but provides any error encountered to the caller.
    pub async fn rollback(mut self) -> Result<(), Error> {
        self.save_point.rollback(&mut self.client).await
    }

    fn new(client: &mut C) -> Transaction<'_, C> {
        Transaction {
            client,
            save_point: SavePoint::default(),
        }
    }

    async fn _save_point(&mut self, name: Option<String>) -> Result<Transaction<'_, C>, Error> {
        let save_point = self.save_point.nest_save_point(&mut self.client, name).await?;
        Ok(Transaction {
            client: self.client,
            save_point,
        })
    }
}

pub struct TransactionOwned<C>
where
    C: ClientBorrowMut,
{
    client: C,
    save_point: SavePoint,
}

impl<C> Drop for TransactionOwned<C>
where
    C: ClientBorrowMut,
{
    fn drop(&mut self) {
        self.save_point.on_drop(&mut self.client);
    }
}

impl<C> TransactionOwned<C>
where
    C: ClientBorrowMut,
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
    pub async fn transaction(&mut self) -> Result<Transaction<'_, C>, Error> {
        self._save_point(None).await
    }

    /// Like [`Client::transaction`], but creates a nested transaction via a savepoint with the specified name.
    ///
    /// [`Client::transaction`]: crate::client::Client::transaction
    pub async fn save_point<I>(&mut self, name: I) -> Result<Transaction<'_, C>, Error>
    where
        I: Into<String>,
    {
        self._save_point(Some(name.into())).await
    }

    /// Consumes the transaction, committing all changes made within it.
    pub async fn commit(mut self) -> Result<(), Error> {
        self.save_point.commit(&mut self.client).await
    }

    /// Rolls the transaction back, discarding all changes made within it.
    ///
    /// This is equivalent to [`Transaction`]'s [`Drop`] implementation, but provides any error encountered to the caller.
    pub async fn rollback(mut self) -> Result<(), Error> {
        self.save_point.rollback(&mut self.client).await
    }

    fn new(client: C) -> Self {
        TransactionOwned {
            client,
            save_point: SavePoint::default(),
        }
    }

    async fn _save_point(&mut self, name: Option<String>) -> Result<Transaction<'_, C>, Error> {
        let save_point = self.save_point.nest_save_point(&mut self.client, name).await?;
        Ok(Transaction {
            client: &mut self.client,
            save_point,
        })
    }
}

impl<'c, C, Q, EO, QO> Execute<&'c Transaction<'_, C>> for Q
where
    C: ClientBorrowMut,
    Q: Execute<&'c Client, ExecuteOutput = EO, QueryOutput = QO>,
{
    type ExecuteOutput = EO;
    type QueryOutput = QO;

    #[inline]
    fn execute(self, cli: &'c Transaction<'_, C>) -> Self::ExecuteOutput {
        Q::execute(self, cli.client.borrow_cli_ref())
    }

    #[inline]
    fn query(self, cli: &'c Transaction<C>) -> Self::QueryOutput {
        Q::query(self, cli.client.borrow_cli_ref())
    }
}

// special treatment for pool connection for it's internal caching logic that are not accessible through Query trait
impl<'c, 'p, Q, EO, QO> Execute<&'c mut Transaction<'_, PoolConnection<'p>>> for Q
where
    Q: Execute<&'c mut PoolConnection<'p>, ExecuteOutput = EO, QueryOutput = QO>,
{
    type ExecuteOutput = EO;
    type QueryOutput = QO;

    #[inline]
    fn execute(self, cli: &'c mut Transaction<'_, PoolConnection<'p>>) -> Self::ExecuteOutput {
        Q::execute(self, &mut *cli.client)
    }

    #[inline]
    fn query(self, cli: &'c mut Transaction<PoolConnection<'p>>) -> Self::QueryOutput {
        Q::query(self, &mut *cli.client)
    }
}

impl<'c, C, Q, EO, QO> Execute<&'c TransactionOwned<C>> for Q
where
    C: ClientBorrowMut,
    Q: Execute<&'c Client, ExecuteOutput = EO, QueryOutput = QO>,
{
    type ExecuteOutput = EO;
    type QueryOutput = QO;

    #[inline]
    fn execute(self, cli: &'c TransactionOwned<C>) -> Self::ExecuteOutput {
        Q::execute(self, cli.client.borrow_cli_ref())
    }

    #[inline]
    fn query(self, cli: &'c TransactionOwned<C>) -> Self::QueryOutput {
        Q::query(self, cli.client.borrow_cli_ref())
    }
}

// special treatment for pool connection for it's internal caching logic that are not accessible through Query trait
impl<'c, 'p, Q, EO, QO> Execute<&'c mut TransactionOwned<PoolConnection<'p>>> for Q
where
    Q: Execute<&'c mut PoolConnection<'p>, ExecuteOutput = EO, QueryOutput = QO>,
{
    type ExecuteOutput = EO;
    type QueryOutput = QO;

    #[inline]
    fn execute(self, cli: &'c mut TransactionOwned<PoolConnection<'p>>) -> Self::ExecuteOutput {
        Q::execute(self, &mut cli.client)
    }

    #[inline]
    fn query(self, cli: &'c mut TransactionOwned<PoolConnection<'p>>) -> Self::QueryOutput {
        Q::query(self, &mut cli.client)
    }
}
