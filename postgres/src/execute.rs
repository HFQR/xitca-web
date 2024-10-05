mod async_impl;
mod sync_impl;

/// Defining how a query is executed. can be used for customizing encoding, executing and database
/// data decoding.
///
/// For customized encoding please see [`Encode`] trait for detail.
/// For customized decoding please see [`IntoResponse`] trait for detail.
///
/// when to use `execute` or `query` methods:
/// - `execute` method is for use case where sql produce an outcome where it only happen once.
///     usually in the form of preparing a statement or observing how many rows have been modified.
/// - `query` method is for use case where sql produce repeated outcome where it can happen multiple times.
///     usually in the form of visiting an iteration of database rows.
///
/// [`Encode`]: crate::driver::codec::encode::Encode
/// [`IntoResponse`]: crate::driver::codec::response::IntoResponse
pub trait Execute<C>
where
    Self: Sized,
{
    /// outcome of execute.
    /// used for single time database response: number of rows affected by execution for example.
    type ExecuteOutput;

    /// outcome of query.
    /// used for repeated database response: database rows for example
    ///
    /// consider impl [`AsyncLendingIterator`] for async iterator of rows
    /// consider impl [`Iterator`] for iterator of rows
    ///
    /// for type of statement where no repeated response will returning this type can point to
    /// [`Execute::ExecuteOutput`] and it's encouraged to make `query` behave identical to `execute`
    ///
    /// [`AsyncLendingIterator`]: crate::iter::AsyncLendingIterator
    type QueryOutput;

    /// define how a statement is executed with single time response
    fn execute(self, cli: C) -> Self::ExecuteOutput;

    /// define how a statement is queried with repeated response
    fn query(self, cli: C) -> Self::QueryOutput;
}

/// blocking version of [`Execute`] for synchronous environment
pub trait ExecuteBlocking<C>
where
    Self: Sized,
{
    type ExecuteOutput;
    type QueryOutput;

    fn execute_blocking(self, cli: C) -> Self::ExecuteOutput;

    fn query_blocking(self, cli: C) -> Self::QueryOutput;
}
