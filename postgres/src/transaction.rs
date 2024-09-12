use crate::query::QuerySimple;

use super::{
    error::Error,
    query::{AsParams, Query, RowStream},
    statement::Statement,
    ToSql,
};

pub struct Transaction<'a, C>
where
    C: Query + QuerySimple,
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
    C: Query + QuerySimple,
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
    C: Query + QuerySimple,
{
    pub(crate) async fn new(client: &mut C) -> Result<Transaction<'_, C>, Error> {
        client._execute_simple("BEGIN").await?;
        Ok(Transaction {
            client,
            save_point: SavePoint::None,
            state: State::WantRollback,
        })
    }

    /// [`Client::query`] for transaction.
    #[inline]
    pub fn query<'a>(&mut self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.client._query(stmt, params)
    }

    /// [`Client::query_raw`] for transaction.
    #[inline]
    pub fn query_raw<'a, I>(&mut self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: AsParams,
    {
        self.client._query_raw(stmt, params)
    }

    /// Like [`Client::transaction`], but creates a nested transaction via a savepoint.
    pub async fn transaction(&mut self) -> Result<Transaction<'_, C>, Error> {
        self._save_point(None).await
    }

    /// Like [`Client::transaction`], but creates a nested transaction via a savepoint with the specified name.
    pub async fn save_point<I>(&mut self, name: I) -> Result<Transaction<'_, C>, Error>
    where
        I: Into<String>,
    {
        self._save_point(Some(name.into())).await
    }

    /// Consumes the transaction, committing all changes made within it.
    pub async fn commit(mut self) -> Result<(), Error> {
        self.state = State::Finish;
        let query = self.save_point.commit_query();
        self.client._execute_simple(&query).await?;
        Ok(())
    }

    /// Rolls the transaction back, discarding all changes made within it.
    ///
    /// This is equivalent to [`Transaction`]'s [`Drop`] implementation, but provides any error encountered to the caller.
    pub async fn rollback(mut self) -> Result<(), Error> {
        self.state = State::Finish;
        let query = self.save_point.rollback_query();
        self.client._execute_simple(&query).await?;
        Ok(())
    }

    async fn _save_point(&mut self, name: Option<String>) -> Result<Transaction<'_, C>, Error> {
        let save_point = self.save_point.nest_save_point(name);
        self.client._execute_simple(&save_point.save_point_query()).await?;

        Ok(Transaction {
            client: self.client,
            save_point,
            state: State::WantRollback,
        })
    }

    fn do_rollback(&mut self) {
        let query = self.save_point.rollback_query();
        drop(self.client._execute_simple(&query));
    }
}
