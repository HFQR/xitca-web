use core::future::Future;

use super::{
    client::Client, error::Error, iter::slice_iter, query::RowStream, statement::Statement, BorrowToSql,
    PoolConnection, ToSql,
};

impl Client {
    pub fn transaction(&mut self) -> impl Future<Output = Result<Transaction<Client>, Error>> {
        Transaction::new(self)
    }
}

pub struct Transaction<'a, C>
where
    C: TransactionOps,
{
    client: &'a mut C,
    state: State,
}

enum State {
    Begin,
    WantRollback,
    Finish,
}

impl<C> Drop for Transaction<'_, C>
where
    C: TransactionOps,
{
    fn drop(&mut self) {
        match self.state {
            State::WantRollback => self.do_rollback(),
            State::Begin | State::Finish => {}
        }
    }
}

impl<C> Transaction<'_, C>
where
    C: TransactionOps,
{
    pub(crate) async fn new(client: &mut C) -> Result<Transaction<'_, C>, Error> {
        let mut tx = Transaction {
            client,
            state: State::Begin,
        };
        tx.begin().await?;
        Ok(tx)
    }

    /// [Client::query] for transaction.
    #[inline]
    pub fn query<'a>(&mut self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.query_raw(stmt, slice_iter(params))
    }

    /// [Client::query_raw] for transaction.
    #[inline]
    pub fn query_raw<'a, I>(&mut self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.client._query_raw(stmt, params)
    }

    pub async fn commit(mut self) -> Result<(), Error> {
        self.client
            ._execute_simple("COMMIT")
            .await
            .map(|_| self.state = State::Finish)
    }

    pub async fn rollback(mut self) -> Result<(), Error> {
        self.client
            ._execute_simple("ROLLBACK")
            .await
            .map(|_| self.state = State::Finish)
    }

    async fn begin(&mut self) -> Result<(), Error> {
        self.client
            ._execute_simple("BEGIN")
            .await
            .map(|_| self.state = State::WantRollback)
    }

    fn do_rollback(&mut self) {
        drop(self.client._execute_simple("ROLLBACK"));
    }
}

pub trait TransactionOps {
    fn _query_raw<'a, I>(&mut self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql;

    fn _execute_simple(&mut self, query: &str) -> impl Future<Output = Result<u64, Error>>;
}

impl TransactionOps for Client {
    fn _query_raw<'a, I>(&mut self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.query_raw(stmt, params)
    }

    fn _execute_simple(&mut self, query: &str) -> impl Future<Output = Result<u64, Error>> {
        self.execute_simple(query)
    }
}

impl TransactionOps for PoolConnection<'_> {
    fn _query_raw<'a, I>(&mut self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.query_raw(stmt, params)
    }

    fn _execute_simple(&mut self, query: &str) -> impl Future<Output = Result<u64, Error>> {
        self.execute_simple(query)
    }
}
