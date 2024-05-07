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
    WantRollback,
    Finish,
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        match self.state {
            State::WantRollback => self.do_rollback(),
            State::Begin | State::Finish => {}
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
        let res = self.client.encode_send_simple("COMMIT")?;
        self.state = State::Finish;
        res.try_into_ready().await
    }

    pub async fn rollback(mut self) -> Result<(), Error> {
        let res = self.client.encode_send_simple("ROLLBACK")?;
        self.state = State::Finish;
        res.try_into_ready().await
    }

    async fn begin(&mut self) -> Result<(), Error> {
        self.client
            .execute_simple("BEGIN")
            .await
            .map(|_| self.state = State::WantRollback)
    }

    fn do_rollback(&mut self) {
        if !self.client.closed() {
            let res = self.client.try_buf_and_split(|b| frontend::query("ROLLBACK", b));
            if let Ok(msg) = res {
                self.client.do_send(msg);
            }
        }
    }
}
