use core::future::Future;

use postgres_protocol::message::backend;

use crate::{
    client::Client,
    column::Column,
    driver::codec::Response,
    error::Error,
    iter::{slice_iter, AsyncLendingIterator},
    query::AsParams,
    row::Row,
    statement::Statement,
    ToSql,
};

use super::row_stream::GenericRowStream;

// TODO: move to client module
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
    pub fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        (&mut &*self)._query(stmt, params)
    }

    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    pub fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: AsParams,
    {
        (&mut &*self)._query_raw(stmt, params)
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
    pub fn execute(
        &self,
        stmt: &Statement,
        params: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<u64, Error>> + Send {
        // TODO: call execute_raw when Rust2024 edition capture rule is stabled.
        let res = self.send_encode(stmt, slice_iter(params));
        async { res?.try_into_row_affected().await }
    }

    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    #[inline]
    pub fn execute_raw<I>(&self, stmt: &Statement, params: I) -> impl Future<Output = Result<u64, Error>> + Send
    where
        I: AsParams,
    {
        let res = self.send_encode(stmt, params);
        async { res?.try_into_row_affected().await }
    }

    // TODO: remove and use Query trait across crate.
    pub(crate) fn send_encode<I>(&self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams,
    {
        (&mut &*self)._send_encode(stmt, params)
    }
}

impl Response {
    pub(crate) async fn try_into_row_affected(mut self) -> Result<u64, Error> {
        let mut rows = 0;
        loop {
            match self.recv().await? {
                backend::Message::RowDescription(_) | backend::Message::DataRow(_) => {}
                backend::Message::CommandComplete(body) => {
                    rows = super::decode::body_to_affected_rows(&body)?;
                }
                backend::Message::EmptyQueryResponse => rows = 0,
                backend::Message::ReadyForQuery(_) => return Ok(rows),
                _ => return Err(Error::unexpected()),
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
                _ => return Err(Error::unexpected()),
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
                backend::Message::BindComplete
                | backend::Message::EmptyQueryResponse
                | backend::Message::CommandComplete(_)
                | backend::Message::PortalSuspended => {}
                backend::Message::ReadyForQuery(_) => return Ok(None),
                _ => return Err(Error::unexpected()),
            }
        }
    }
}

/// trait generic over api used for querying with typed statement.
pub trait Query {
    /// query with statement and dynamic typed params
    #[inline]
    fn _query<'a>(&mut self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self._query_raw(stmt, slice_iter(params))
    }

    /// query with statement and single typed params
    #[inline]
    fn _query_raw<'a, I>(&mut self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: AsParams,
    {
        self._send_encode(stmt, params).map(|res| RowStream {
            res,
            col: stmt.columns(),
            ranges: Vec::new(),
        })
    }

    /// encode statement and params and send it to client driver
    fn _send_encode<I>(&mut self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams;
}

// TODO: move to client module
impl Query for &Client {
    #[inline]
    fn _send_encode<I>(&mut self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams,
    {
        send_encode(self, stmt, params)
    }
}

// TODO: move to client module
impl Query for Client {
    #[inline]
    fn _send_encode<I>(&mut self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams,
    {
        send_encode(self, stmt, params)
    }
}

// TODO: move to client module
fn send_encode<I>(cli: &Client, stmt: &Statement, params: I) -> Result<Response, Error>
where
    I: AsParams,
{
    cli.tx.send(|buf| super::encode::encode(buf, stmt, params.into_iter()))
}
