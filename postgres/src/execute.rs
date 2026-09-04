mod async_impl;
mod sync_impl;

use core::{future::Future, pin::Pin};

use crate::error::Error;

/// Defining how a query is executed. can be used for customizing encoding, executing and database
/// data decoding.
///
/// For customized encoding please see [`Encode`] trait for detail.
/// For customized decoding please see [`IntoResponse`] trait for detail.
///
/// when to use `execute` or `query` methods:
/// - `execute` method is for use case where sql produce an outcome where it only happen once.
///   usually in the form of preparing a statement or observing how many rows have been modified.
/// - `query` method is for use case where sql produce repeated outcome where it can happen multiple times.
///   usually in the form of visiting an iteration of database rows.
///
/// [`Encode`]: crate::driver::codec::encode::Encode
/// [`IntoResponse`]: crate::driver::codec::response::IntoResponse
pub trait Execute<C>
where
    Self: Sized,
{
    /// item produced by execute.
    /// used for single time database response: number of rows affected by execution for example.
    type ExecuteOutput;

    /// item produced by query.
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
    fn execute(self, cli: C) -> impl Future<Output = Result<Self::ExecuteOutput, Error>> + Send;

    /// define how a statement is queried with repeated response
    fn query(self, cli: C) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send;
}

/// object safe variant of [`Execute`].
///
/// [`Execute`] returns an opaque future so it can not be used through a trait object. this trait
/// boxes the future instead which makes it usable as `dyn ExecuteDyn`. it's implemented for every
/// [`Execute`] type so nothing has to be written by hand.
///
/// prefer [`Execute`] where the concrete type is known: it does not allocate.
///
/// # Examples
/// ```rust
/// # use xitca_postgres::{Client, ExecuteDyn, RowStreamOwned};
/// // a collection of queries with unrelated types can share one boxed type.
/// fn collect<'c>(
///     queries: Vec<Box<dyn ExecuteDyn<'c, &'c Client, ExecuteOutput = u64, QueryOutput = RowStreamOwned> + Send>>,
/// ) -> Vec<Box<dyn ExecuteDyn<'c, &'c Client, ExecuteOutput = u64, QueryOutput = RowStreamOwned> + Send>> {
///     queries
/// }
/// ```
pub trait ExecuteDyn<'c, C>
where
    C: 'c,
{
    /// item produced by [`ExecuteDyn::execute_dyn`]
    type ExecuteOutput;

    /// item produced by [`ExecuteDyn::query_dyn`]
    type QueryOutput;

    /// object safe variant of [`Execute::execute`]
    fn execute_dyn(
        self: Box<Self>,
        cli: C,
    ) -> Pin<Box<dyn Future<Output = Result<Self::ExecuteOutput, Error>> + Send + 'c>>;

    /// object safe variant of [`Execute::query`]
    fn query_dyn(
        self: Box<Self>,
        cli: C,
    ) -> Pin<Box<dyn Future<Output = Result<Self::QueryOutput, Error>> + Send + 'c>>;
}

impl<'c, C, E> ExecuteDyn<'c, C> for E
where
    E: Execute<C> + Send + 'c,
    C: Send + 'c,
{
    type ExecuteOutput = E::ExecuteOutput;
    type QueryOutput = E::QueryOutput;

    #[inline]
    fn execute_dyn(
        self: Box<Self>,
        cli: C,
    ) -> Pin<Box<dyn Future<Output = Result<Self::ExecuteOutput, Error>> + Send + 'c>> {
        Box::pin(async move { (*self).execute(cli).await })
    }

    #[inline]
    fn query_dyn(
        self: Box<Self>,
        cli: C,
    ) -> Pin<Box<dyn Future<Output = Result<Self::QueryOutput, Error>> + Send + 'c>> {
        Box::pin(async move { (*self).query(cli).await })
    }
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
