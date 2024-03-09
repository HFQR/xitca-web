use postgres_protocol::message::backend;

use xitca_io::bytes::BytesMut;

use crate::{
    client::Client,
    column::Column,
    driver::Response,
    error::Error,
    iter::{slice_iter, AsyncLendingIterator},
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
        let buf = self.encode(stmt, params)?;
        self.query_buf(stmt, buf).await
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
        let buf = self.encode(stmt, params)?;
        self.send_buf(buf).await?.try_into_row_affected().await
    }

    pub(crate) async fn query_buf<'a>(&self, stmt: &'a Statement, buf: BytesMut) -> Result<RowStream<'a>, Error> {
        self.send_buf(buf).await.map(|res| RowStream {
            col: stmt.columns(),
            res,
            ranges: Vec::new(),
        })
    }

    async fn send_buf(&self, buf: BytesMut) -> Result<Response, Error> {
        let mut res = self.send(buf).await?;
        match res.recv().await? {
            backend::Message::BindComplete => Ok(res),
            _ => Err(Error::UnexpectedMessage),
        }
    }

    fn encode<I>(&self, stmt: &Statement, params: I) -> Result<BytesMut, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        let params = params.into_iter();
        stmt.params_assert(&params);
        self.try_buf_and_split(|buf| super::encode::encode(buf, stmt, params))
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

impl<'a> AsyncLendingIterator for RowStream<'a> {
    type Ok<'i> = Row<'i> where Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        loop {
            match self.res.recv().await? {
                backend::Message::DataRow(body) => return Row::try_new(self.col, body, &mut self.ranges).map(Some),
                backend::Message::EmptyQueryResponse
                | backend::Message::CommandComplete(_)
                | backend::Message::PortalSuspended => {}
                backend::Message::ReadyForQuery(_) => return Ok(None),
                _ => return Err(Error::UnexpectedMessage),
            }
        }
    }
}
