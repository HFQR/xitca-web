use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use super::{
    driver::codec::AsParams,
    error::Error,
    prepare::Prepare,
    query::{Query, RowAffected, RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{
        Statement, StatementCreate, StatementCreateBlocking, StatementGuarded, StatementNamed, StatementQuery,
        StatementUnnamedBind, StatementUnnamedQuery,
    },
    BoxedFuture,
};

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
pub trait Execute<'c, C>
where
    Self: Sized,
{
    /// async outcome of execute.
    type ExecuteOutput: Future;
    /// iterator outcome of query.
    ///
    /// by default this type should be matching `C`'s [`Query::_query`] output type.
    ///
    /// consider impl [`AsyncLendingIterator`] for async iterator of rows
    /// consider impl [`Iterator`] for iterator of rows
    ///
    /// [`AsyncLendingIterator`]: crate::iter::AsyncLendingIterator
    type QueryOutput;

    /// define how a query is executed with async outcome.
    fn execute(self, cli: &'c C) -> Self::ExecuteOutput;

    /// define how a query is executed with iterator of database rows as return type.
    fn query(self, cli: &'c C) -> Self::QueryOutput;

    /// blocking version of [`Execute::execute`]
    fn execute_blocking(self, cli: &'c C) -> <Self::ExecuteOutput as Future>::Output;
}

/// mutable version of [`Execute`] trait where C type is mutably borrowed
pub trait ExecuteMut<'c, C>
where
    Self: Sized,
{
    type ExecuteMutOutput: Future;
    type QueryMutOutput;

    fn execute_mut(self, cli: &'c mut C) -> Self::ExecuteMutOutput;

    fn query_mut(self, cli: &'c mut C) -> Self::QueryMutOutput;

    fn execute_mut_blocking(self, cli: &'c mut C) -> <Self::ExecuteMutOutput as Future>::Output;
}

impl<'s, C> Execute<'_, C> for &'s Statement
where
    C: Query,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Result<RowStream<'s>, Error>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteOutput {
        self.query(cli).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &C) -> Self::QueryOutput {
        cli._query(self)
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        let stream = self.query(cli)?;
        RowAffected::from(stream).wait()
    }
}

impl<'s, C> Execute<'_, C> for &'s str
where
    C: Query,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Result<RowSimpleStream, Error>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteOutput {
        self.query(cli).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &C) -> Self::QueryOutput {
        cli._query(self)
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        let stream = self.query(cli)?;
        RowAffected::from(stream).wait()
    }
}

type IntoGuardedFuture<'c, C> = IntoGuarded<'c, BoxedFuture<'c, Result<Statement, Error>>, C>;

pub struct IntoGuarded<'a, F, C> {
    fut: F,
    cli: &'a C,
}

impl<'a, F, C> Future for IntoGuarded<'a, F, C>
where
    F: Future<Output = Result<Statement, Error>> + Unpin,
    C: Prepare,
{
    type Output = Result<StatementGuarded<'a, C>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut this.fut)
            .poll(cx)
            .map_ok(|stmt| stmt.into_guarded(this.cli))
    }
}

impl<'c, 's, C> Execute<'c, C> for StatementNamed<'s>
where
    C: Query + Prepare + 'c,
    's: 'c,
{
    type ExecuteOutput = ResultFuture<IntoGuardedFuture<'c, C>>;
    type QueryOutput = Self::ExecuteOutput;

    #[inline]
    fn execute(self, cli: &'c C) -> Self::ExecuteOutput {
        cli._query(StatementCreate::from((self, cli)))
            .map(|fut| IntoGuarded { fut, cli })
            .into()
    }

    #[inline]
    fn query(self, cli: &'c C) -> Self::QueryOutput {
        self.execute(cli)
    }

    #[inline]
    fn execute_blocking(self, cli: &'c C) -> Result<StatementGuarded<'c, C>, Error> {
        let stmt = cli._query(StatementCreateBlocking::from((self, cli)))??;
        Ok(stmt.into_guarded(cli))
    }
}

impl<'s, C, P> Execute<'_, C> for StatementQuery<'s, P>
where
    C: Query + Prepare,
    P: AsParams + 's,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Result<RowStream<'s>, Error>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteOutput {
        self.query(cli).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &C) -> Self::QueryOutput {
        cli._query(self)
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        let stream = self.query(cli)?;
        RowAffected::from(stream).wait()
    }
}

impl<'s, 'c, C, P> Execute<'c, C> for StatementUnnamedBind<'s, P>
where
    C: Query + Prepare + 'c,
    P: AsParams + 'c,
    's: 'c,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Result<RowStreamGuarded<'c, C>, Error>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteOutput {
        self.query(cli).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &'c C) -> Self::QueryOutput {
        cli._query(StatementUnnamedQuery::from((self, cli)))
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        let stream = self.query(cli)?;
        RowAffected::from(stream).wait()
    }
}

pub struct ResultFuture<F>(Result<F, Option<Error>>);

impl<F> From<Result<F, Error>> for ResultFuture<F> {
    fn from(res: Result<F, Error>) -> Self {
        Self(res.map_err(Some))
    }
}

impl<F, T> Future for ResultFuture<F>
where
    F: Future<Output = Result<T, Error>> + Unpin,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut().0 {
            Ok(ref mut res) => Pin::new(res).poll(cx),
            Err(ref mut e) => Poll::Ready(Err(e.take().unwrap())),
        }
    }
}
