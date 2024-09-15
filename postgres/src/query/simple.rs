use core::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend;

use crate::{
    column::Column, driver::codec::Response, error::Error, iter::AsyncLendingIterator, row::RowSimple, types::Type,
};

use super::row_stream::GenericRowStream;

/// trait generic over api used for querying with non typed string query without preparing.
///
/// types like [Transaction] and [CopyIn] accept generic client type and they are able to use user supplied
/// client new type to operate and therefore reduce less new types and methods boilerplate.
///
/// [Transaction]: crate::transaction::Transaction
/// [CopyIn]: crate::copy::CopyIn
pub trait QuerySimple {
    #[inline]
    fn _query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        self._send_encode_simple(stmt).map(|res| RowSimpleStream {
            res,
            col: Vec::new(),
            ranges: Vec::new(),
        })
    }

    fn _execute_simple(&self, stmt: &str) -> ExecuteSimple {
        let res = self._send_encode_simple(stmt);
        // TODO:
        // use async { res?.try_into_row_affected().await } with Rust 2024 edition
        ExecuteSimple {
            res: Some(res),
            rows_affected: 0,
        }
    }

    fn _send_encode_simple(&self, stmt: &str) -> Result<Response, Error>;
}

pub struct ExecuteSimple {
    res: Option<Result<Response, Error>>,
    rows_affected: u64,
}

impl Future for ExecuteSimple {
    type Output = Result<u64, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this.res.as_mut().expect("ExecuteSimple is polled after finish") {
            Ok(res) => {
                ready!(res.poll_try_into_ready(&mut this.rows_affected, cx))?;
                Poll::Ready(Ok(this.rows_affected))
            }
            Err(_) => Poll::Ready(this.res.take().unwrap().map(|_| 0)),
        }
    }
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
