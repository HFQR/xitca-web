use core::future::Future;

use postgres_protocol::message::backend;

use crate::{
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

/// trait generic over api used for querying with typed prepared statement.
///
/// types like [Transaction] and [CopyIn] accept generic client type and they are able to use user supplied
/// client new type to operate and therefore reduce less new types and methods boilerplate.
///
/// [Transaction]: crate::transaction::Transaction
/// [CopyIn]: crate::copy::CopyIn
pub trait Query {
    /// query with statement and dynamic typed params
    #[inline]
    fn _query<'a>(&mut self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self._query_raw(stmt, slice_iter(params))
    }

    /// flexible version of [Query::_query]
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

    /// query that don't return any row but number of rows affected by it
    #[inline]
    fn _execute(
        &mut self,
        stmt: &Statement,
        params: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<u64, Error>> + Send {
        self._execute_raw(stmt, slice_iter(params))
    }

    /// flexible version of [Query::_execute]
    fn _execute_raw<I>(&mut self, _: &Statement, _: I) -> impl Future<Output = Result<u64, Error>> + Send
    where
        I: AsParams,
    {
        async { todo!("Waiting for Rust 2024 Edition") }
    }

    /// encode statement and params and send it to client driver
    fn _send_encode<I>(&mut self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams;
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
