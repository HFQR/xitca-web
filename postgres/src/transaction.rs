use postgres_protocol::message::frontend;

use super::{client::Client, error::Error, query::RowStream, statement::Statement, BorrowToSql, ToSql};

impl Client {
    pub async fn transaction(&mut self) -> Result<Transaction<'_>, Error> {
        let mut tx = Transaction {
            client: self,
            state: State::Begin,
        };
        tx.begin().await?;
        Ok(tx)
    }
}

pub struct Transaction<'a> {
    client: &'a mut Client,
    state: State,
}

enum State {
    Begin,
    Finish,
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        match self.state {
            State::Begin => self.cancel(),
            State::Finish => {}
        }
    }
}

impl Transaction<'_> {
    /// [Client::query] for transaction.
    #[inline]
    pub async fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.client.query(stmt, params).await
    }

    /// [Client::query_raw] for transaction.
    #[inline]
    pub async fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.client.query_raw(stmt, params).await
    }

    pub async fn commit(mut self) -> Result<(), Error> {
        self.state = State::Finish;
        self.client.execute_simple("COMMIT").await.map(|_| ())
    }

    pub async fn rollback(mut self) -> Result<(), Error> {
        self.state = State::Finish;
        self.client.execute_simple("ROLLBACK").await.map(|_| ())
    }

    async fn begin(&mut self) -> Result<(), Error> {
        self.client.execute_simple("BEGIN").await.map(|_| ())
    }

    fn cancel(&mut self) {
        if !self.client.closed() {
            let res = self.client.try_buf_and_split(|b| frontend::query("ROLLBACK", b));
            if let Ok(msg) = res {
                self.client.do_send(msg);
            }
        }
    }
}
