use core::future::Future;

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::{backend, frontend};

use crate::{
    client::Client, column::Column, driver::codec::Response, error::Error, iter::AsyncLendingIterator, row::RowSimple,
    Type,
};

use super::row_stream::GenericRowStream;

// TODO: move to client module
impl Client {
    #[inline]
    pub fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        (&mut &*self)._query_simple(stmt)
    }

    pub fn execute_simple(&self, stmt: &str) -> impl Future<Output = Result<u64, Error>> {
        let res = self.send_encode_simple(stmt);
        async { res?.try_into_row_affected().await }
    }

    // TODO: remove and use QuerySimple trait across crate.
    pub(crate) fn send_encode_simple(&self, stmt: &str) -> Result<Response, Error> {
        (&mut &*self)._send_encode_simple(stmt)
    }
}

/// trait generic over api used for querying with non typed string query without preparing.
///
/// types like [Transaction] and [CopyIn] accept generic client type and they are able to use user supplied
/// client new type to operate and therefore reduce less new types and methods boilerplate.
///
/// [Transaction]: crate::transaction::Transaction
/// [CopyIn]: crate::copy::CopyIn
pub trait QuerySimple {
    #[inline]
    fn _query_simple(&mut self, stmt: &str) -> Result<RowSimpleStream, Error> {
        self._send_encode_simple(stmt).map(|res| RowSimpleStream {
            res,
            col: Vec::new(),
            ranges: Vec::new(),
        })
    }

    fn _execute_simple(&self, _: &str) -> impl Future<Output = Result<u64, Error>> {
        async { todo!("Waiting for Rust 2024 Edition") }
    }

    fn _send_encode_simple(&mut self, stmt: &str) -> Result<Response, Error>;
}

// TODO: move to client module
impl QuerySimple for Client {
    fn _send_encode_simple(&mut self, stmt: &str) -> Result<Response, Error> {
        send_encode_simple(self, stmt)
    }
}

// TODO: move to client module
impl QuerySimple for &Client {
    fn _send_encode_simple(&mut self, stmt: &str) -> Result<Response, Error> {
        send_encode_simple(self, stmt)
    }
}

// TODO: move to client module
fn send_encode_simple(cli: &Client, stmt: &str) -> Result<Response, Error> {
    cli.tx.send(|buf| frontend::query(stmt, buf).map_err(Into::into))
}

/// A stream of simple query results.
pub type RowSimpleStream = GenericRowStream<Vec<Column>>;

impl AsyncLendingIterator for RowSimpleStream {
    type Ok<'i> = RowSimple<'i> where Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        loop {
            match self.res.recv().await? {
                backend::Message::RowDescription(body) => {
                    self.col = body
                        .fields()
                        // text type is used to match RowSimple::try_get's implementation
                        // where column's pg type is always assumed as Option<&str>.
                        // (no runtime pg type check so this does not really matter. it's
                        // better to keep the type consistent though)
                        .map(|f| Ok(Column::new(f.name(), Type::TEXT)))
                        .collect::<Vec<_>>()?;
                }
                backend::Message::DataRow(body) => {
                    return RowSimple::try_new(&self.col, body, &mut self.ranges).map(Some);
                }
                backend::Message::CommandComplete(_)
                | backend::Message::EmptyQueryResponse
                | backend::Message::ReadyForQuery(_) => return Ok(None),
                _ => return Err(Error::unexpected()),
            }
        }
    }
}
