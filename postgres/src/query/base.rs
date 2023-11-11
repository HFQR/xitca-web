use postgres_protocol::message::backend;

use crate::{
    client::Client,
    column::Column,
    driver::Response,
    error::Error,
    iter::{slice_iter, AsyncIterator},
    row::Row,
    statement::Statement,
    BorrowToSql, ToSql,
};

use super::row_stream::GenericRowStream;

impl Client {
    /// Executes a statement, returning a vector of the resulting rows.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the
    /// parameter of the list provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters),
    /// consider preparing the statement up front with the [Client::prepare] method.
    ///
    /// # Panics
    ///
    /// Panics if given params slice length does not match the length of [Statement::params].
    #[inline]
    pub async fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.query_raw(stmt, slice_iter(params)).await
    }

    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    pub async fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.encode_send(stmt, params).await.map(|res| RowStream {
            col: stmt.columns(),
            res,
            ranges: Vec::new(),
        })
    }

    /// Executes a statement, returning the number of rows modified.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters),
    /// consider preparing the statement up front with the [Client::prepare] method.
    ///
    /// If the statement does not modify any rows (e.g. `SELECT`), 0 is returned.
    ///
    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    #[inline]
    pub async fn execute(&self, stmt: &Statement, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error> {
        self.execute_raw(stmt, slice_iter(params)).await
    }

    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    #[inline]
    pub async fn execute_raw<I>(&self, stmt: &Statement, params: I) -> Result<u64, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.encode_send(stmt, params).await?.try_into_row_affected().await
    }

    async fn encode_send<I>(&self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        let params = params.into_iter();
        stmt.params_assert(&params);
        let buf = self.try_buf_and_split(|buf| super::encode::encode(buf, stmt, params))?;
        let mut res = self.send(buf).await?;
        match res.recv().await? {
            backend::Message::BindComplete => Ok(res),
            _ => Err(Error::UnexpectedMessage),
        }
    }
}

impl Response {
    pub(super) async fn try_into_row_affected(mut self) -> Result<u64, Error> {
        let mut rows = 0;
        loop {
            match self.recv().await? {
                backend::Message::RowDescription(_) | backend::Message::DataRow(_) => {}
                backend::Message::CommandComplete(body) => {
                    rows = super::decode::body_to_affected_rows(&body)?;
                }
                backend::Message::EmptyQueryResponse => rows = 0,
                backend::Message::ReadyForQuery(_) => return Ok(rows),
                _ => return Err(Error::UnexpectedMessage),
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn try_into_ready(mut self) -> Result<(), Error> {
        loop {
            match self.recv().await? {
                backend::Message::RowDescription(_)
                | backend::Message::DataRow(_)
                | backend::Message::EmptyQueryResponse => {}
                backend::Message::ReadyForQuery(_) => return Ok(()),
                _ => return Err(Error::UnexpectedMessage),
            }
        }
    }
}

/// A stream of table rows.
pub type RowStream<'a> = GenericRowStream<&'a [Column]>;

impl<'a> AsyncIterator for RowStream<'a> {
    type Item<'i> = Result<Row<'i>, Error> where 'a: 'i;

    async fn next(&mut self) -> Option<Self::Item<'_>> {
        loop {
            match self.res.recv().await {
                Ok(msg) => match msg {
                    backend::Message::DataRow(body) => return Some(Row::try_new(self.col, body, &mut self.ranges)),
                    backend::Message::EmptyQueryResponse
                    | backend::Message::CommandComplete(_)
                    | backend::Message::PortalSuspended => {}
                    backend::Message::ReadyForQuery(_) => return None,
                    _ => return Some(Err(Error::UnexpectedMessage)),
                },
                Err(e) => return Some(Err(e)),
            }
        }
    }
}
