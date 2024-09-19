mod stream;

#[cfg(feature = "compat")]
pub(crate) mod compat;

pub use stream::{RowSimpleStream, RowStream};

use core::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use super::{
    driver::codec::{AsParams, Encode, Response},
    error::Error,
    iter::slice_iter,
    types::ToSql,
};

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
    fn _query<'a, S>(&self, stmt: &'a S, params: &[&(dyn ToSql + Sync)]) -> Result<S::RowStream<'a>, Error>
    where
        S: Encode + ?Sized,
    {
        self._query_raw(stmt, slice_iter(params))
    }

    /// flexible version of [Query::_query]
    #[inline]
    fn _query_raw<'a, S, I>(&self, stmt: &'a S, params: I) -> Result<S::RowStream<'a>, Error>
    where
        S: Encode + ?Sized,
        I: AsParams,
    {
        self._send_encode_query(stmt, params).map(|res| stmt.row_stream(res))
    }

    /// query that don't return any row but number of rows affected by it
    #[inline]
    fn _execute<S>(&self, stmt: &S, params: &[&(dyn ToSql + Sync)]) -> ExecuteFuture
    where
        S: Encode + ?Sized,
    {
        self._execute_raw(stmt, slice_iter(params))
    }

    /// flexible version of [Query::_execute]
    fn _execute_raw<S, I>(&self, stmt: &S, params: I) -> ExecuteFuture
    where
        S: Encode + ?Sized,
        I: AsParams,
    {
        let res = self._send_encode_query(stmt, params);
        // TODO:
        // use async { res?.try_into_row_affected().await } with Rust 2024 edition
        ExecuteFuture {
            res: res.map_err(Some),
            rows_affected: 0,
        }
    }

    /// encode statement and params and send it to client driver
    fn _send_encode_query<S, I>(&self, stmt: &S, params: I) -> Result<Response, Error>
    where
        S: Encode + ?Sized,
        I: AsParams;
}

pub struct ExecuteFuture {
    res: Result<Response, Option<Error>>,
    rows_affected: u64,
}

impl Future for ExecuteFuture {
    type Output = Result<u64, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this.res {
            Ok(ref mut res) => {
                ready!(res.poll_try_into_ready(&mut this.rows_affected, cx))?;
                Poll::Ready(Ok(this.rows_affected))
            }
            Err(ref mut e) => Poll::Ready(Err(e.take().expect("ExecuteFuture polled after finish"))),
        }
    }
}
