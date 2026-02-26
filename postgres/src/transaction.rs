mod builder;
mod portal;

use core::ops::{Deref, DerefMut};

use std::borrow::Cow;

use super::{
    client::{Client, ClientBorrowMut},
    driver::codec::AsParams,
    error::Error,
    execute::Execute,
    pool::{GenericPoolConnection, PermitLike},
    statement::Statement,
    types::ToSql,
};

pub use builder::{IsolationLevel, TransactionBuilder};
pub use portal::Portal;

struct SavePoint {
    name: Option<String>,
    depth: u32,
    state: State,
}

enum State {
    WantRollback,
    Finish,
}

impl Default for SavePoint {
    fn default() -> Self {
        Self {
            name: None,
            depth: 0,
            state: State::WantRollback,
        }
    }
}

impl SavePoint {
    // rollback runs in Drop trait impl. the execution part has to run eargerly
    fn rollback(&mut self, cli: impl ClientBorrowMut) -> impl Future<Output = Result<(), Error>> + Send {
        self.state = State::Finish;

        let fut = match self.depth {
            0 => Cow::Borrowed("ROLLBACK"),
            depth => match self.name {
                None => Cow::Owned(format!("ROLLBACK TO sp_{depth}")),
                Some(ref name) => Cow::Owned(format!("ROLLBACK TO {name}")),
            },
        }
        .execute(cli.borrow_cli_ref());

        async { fut.await.map(|_| ()) }
    }

    async fn commit(&mut self, cli: impl ClientBorrowMut) -> Result<(), Error> {
        self.state = State::Finish;

        match self.depth {
            0 => Cow::Borrowed("COMMIT"),
            depth => match self.name {
                None => Cow::Owned(format!("RELEASE sp_{depth}")),
                Some(ref name) => Cow::Owned(format!("RELEASE {name}")),
            },
        }
        .execute(cli.borrow_cli_ref())
        .await
        .map(|_| ())
    }

    async fn nest_save_point(&self, cli: impl ClientBorrowMut, name: Option<String>) -> Result<Self, Error> {
        let depth = self.depth + 1;

        match self.depth {
            0 => match name {
                Some(ref name) => Cow::Owned(format!("SAVEPOINT {name}")),
                None => Cow::Borrowed("SAVEPOINT sp_1"),
            },
            depth => match name {
                Some(ref name) => Cow::Owned(format!("SAVEPOINT {name}")),
                None => Cow::Owned(format!("SAVEPOINT sp_{depth}")),
            },
        }
        .execute(cli.borrow_cli_ref())
        .await
        .map(|_| SavePoint {
            name,
            depth,
            state: State::WantRollback,
        })
    }

    fn on_drop(&mut self, cli: impl ClientBorrowMut) {
        if matches!(self.state, State::WantRollback) {
            drop(self.rollback(cli));
        }
    }
}

pub struct Transaction<'a, C>
where
    C: ClientBorrowMut,
{
    client: _Client<'a, C>,
    save_point: SavePoint,
}

enum _Client<'a, C> {
    Owned(C),
    Borrowed(&'a mut C),
}

impl<C> _Client<'_, C> {
    #[inline]
    fn reborrow(&mut self) -> _Client<'_, C> {
        _Client::Borrowed(self.deref_mut())
    }
}

impl<C> Deref for _Client<'_, C> {
    type Target = C;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(c) => c,
            Self::Owned(c) => c,
        }
    }
}

impl<C> DerefMut for _Client<'_, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Borrowed(c) => c,
            Self::Owned(c) => c,
        }
    }
}

impl<C> Drop for Transaction<'_, C>
where
    C: ClientBorrowMut,
{
    fn drop(&mut self) {
        self.save_point.on_drop(self.client.deref_mut());
    }
}

impl<C> Transaction<'_, C>
where
    C: ClientBorrowMut,
{
    /// A maximally flexible version of [`Transaction::bind`].
    pub async fn bind<'p, I>(&'p self, statement: &'p Statement, params: I) -> Result<Portal<'p, C>, Error>
    where
        I: AsParams,
    {
        Portal::new(&*self.client, statement, params).await
    }

    /// Binds a statement to a set of parameters, creating a [`Portal`] which can be incrementally queried.
    ///
    /// Portals only last for the duration of the transaction in which they are created, and can only be used on the
    /// connection that created them.
    pub async fn bind_dyn<'p>(
        &'p self,
        statement: &'p Statement,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Portal<'p, C>, Error> {
        self.bind(statement, params.iter().cloned()).await
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
        self.save_point.commit(self.client.deref_mut()).await
    }

    /// Rolls the transaction back, discarding all changes made within it.
    ///
    /// This is equivalent to [`Transaction`]'s [`Drop`] implementation, but provides any error encountered to the caller.
    pub async fn rollback(mut self) -> Result<(), Error> {
        self.save_point.rollback(self.client.deref_mut()).await
    }

    fn new(client: &mut C) -> Transaction<'_, C> {
        Transaction {
            client: _Client::Borrowed(client),
            save_point: SavePoint::default(),
        }
    }

    fn new_owned<'a>(client: C) -> Transaction<'a, C>
    where
        C: 'a,
    {
        Transaction {
            client: _Client::Owned(client),
            save_point: SavePoint::default(),
        }
    }

    async fn _save_point(&mut self, name: Option<String>) -> Result<Transaction<'_, C>, Error> {
        let save_point = self.save_point.nest_save_point(self.client.deref_mut(), name).await?;
        Ok(Transaction {
            client: self.client.reborrow(),
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
impl<'c, P, Q, EO, QO> Execute<&'c mut Transaction<'_, GenericPoolConnection<P>>> for Q
where
    P: PermitLike,
    Q: Execute<&'c mut GenericPoolConnection<P>, ExecuteOutput = EO, QueryOutput = QO>,
{
    type ExecuteOutput = EO;
    type QueryOutput = QO;

    #[inline]
    fn execute(self, cli: &'c mut Transaction<'_, GenericPoolConnection<P>>) -> Self::ExecuteOutput {
        Q::execute(self, cli.client.deref_mut())
    }

    #[inline]
    fn query(self, cli: &'c mut Transaction<GenericPoolConnection<P>>) -> Self::QueryOutput {
        Q::query(self, cli.client.deref_mut())
    }
}
