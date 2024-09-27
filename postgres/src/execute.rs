use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{
    driver::codec::encode::{ExecuteEncode, QueryEncode},
    statement::{StatementCreate, StatementGuarded},
};

use super::{
    driver::codec::{response::RowAffected, AsParams},
    error::Error,
    prepare::Prepare,
    query::{Query, RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{
        Statement, StatementCreateBlocking, StatementNamed, StatementQuery, StatementUnnamedBind, StatementUnnamedQuery,
    },
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
    C: Query,
    Self: Sized,
{
    /// async outcome of execute.
    type ExecuteFuture: Future;
    /// iterator outcome of query.
    ///
    /// by default this type should be matching `C`'s [`Query::_query`] output type.
    ///
    /// consider impl [`AsyncLendingIterator`] for async iterator of rows
    /// consider impl [`Iterator`] for iterator of rows
    ///
    /// [`AsyncLendingIterator`]: crate::iter::AsyncLendingIterator
    type RowStream;

    /// define how a query is executed with async outcome.
    fn execute(self, cli: &'c C) -> Self::ExecuteFuture;

    /// define how a query is executed with iterator of database rows as return type.
    fn query(self, cli: &'c C) -> Result<Self::RowStream, Error>;

    /// blocking version of [`Execute::execute`]
    fn execute_blocking(self, cli: &'c C) -> <Self::ExecuteFuture as Future>::Output;
}

impl<'s, C> Execute<'_, C> for &'s Statement
where
    C: Query,
{
    type ExecuteFuture = ResultFuture<RowAffected>;
    type RowStream = RowStream<'s>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteFuture {
        cli._query(ExecuteEncode(self)).into()
    }

    #[inline]
    fn query(self, cli: &C) -> Result<Self::RowStream, Error> {
        cli._query(QueryEncode(self))
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        cli._query(ExecuteEncode(self))?.wait()
    }
}

impl<'s, C> Execute<'_, C> for &'s str
where
    C: Query,
{
    type ExecuteFuture = ResultFuture<RowAffected>;
    type RowStream = RowSimpleStream;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteFuture {
        cli._query(ExecuteEncode(self)).into()
    }

    #[inline]
    fn query(self, cli: &C) -> Result<Self::RowStream, Error> {
        cli._query(QueryEncode(self))
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        cli._query(ExecuteEncode(self))?.wait()
    }
}

impl<'c, 's, C> Execute<'c, C> for StatementNamed<'s>
where
    C: Query + Prepare + 'c,
    's: 'c,
{
    type ExecuteFuture =
        ResultFuture<Pin<Box<dyn Future<Output = Result<StatementGuarded<'c, C>, Error>> + Send + 'c>>>;
    type RowStream = Self::ExecuteFuture;

    #[inline]
    fn execute(self, cli: &'c C) -> Self::ExecuteFuture {
        cli._query(StatementCreate::from((self, cli)))
            .map(|fut| Box::pin(async { fut.await.map(|stmt| stmt.into_guarded(cli)) }) as _)
            .into()
    }

    #[inline]
    fn query(self, cli: &'c C) -> Result<Self::RowStream, Error> {
        Ok(self.execute(cli))
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
    type ExecuteFuture = ResultFuture<RowAffected>;
    type RowStream = RowStream<'s>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteFuture {
        cli._query(ExecuteEncode(self)).into()
    }

    #[inline]
    fn query(self, cli: &C) -> Result<Self::RowStream, Error> {
        cli._query(QueryEncode(self))
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        cli._query(ExecuteEncode(self))?.wait()
    }
}

impl<'s, 'c, C, P> Execute<'c, C> for StatementUnnamedBind<'s, P>
where
    C: Query + Prepare + 'c,
    P: AsParams + 's + 'c,
    's: 'c,
{
    type ExecuteFuture = ResultFuture<RowAffected>;
    type RowStream = RowStreamGuarded<'c, C>;

    #[inline]
    fn execute(self, cli: &'c C) -> Self::ExecuteFuture {
        cli._query(ExecuteEncode(StatementUnnamedQuery::from((self, cli))))
            .into()
    }

    #[inline]
    fn query(self, cli: &'c C) -> Result<Self::RowStream, Error> {
        cli._query(QueryEncode(StatementUnnamedQuery::from((self, cli))))
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        cli._query(ExecuteEncode(StatementUnnamedQuery::from((self, cli))))?
            .wait()
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

#[cfg(test)]
mod test {
    use core::{
        future::{Future, IntoFuture},
        pin::Pin,
    };

    use crate::Postgres;

    use super::*;

    #[tokio::test]
    async fn execute_with_lifetime() {
        struct ExecuteCaptureClient<'s>(&'s str);

        impl<'c, 's, C> Execute<'c, C> for ExecuteCaptureClient<'s>
        where
            C: Query + Prepare,
            's: 'c,
        {
            type ExecuteFuture = Pin<Box<dyn Future<Output = Result<u64, Error>> + Send + 'c>>;
            type RowStream = RowSimpleStream;

            fn execute(self, cli: &'c C) -> Self::ExecuteFuture {
                Box::pin(async move {
                    let stmt = Statement::named(self.0, &[]).execute(cli).await?;
                    stmt.execute(cli).await
                })
            }

            fn query(self, _: &'c C) -> Result<Self::RowStream, Error> {
                todo!()
            }

            fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
                Statement::named(self.0, &[])
                    .execute_blocking(cli)?
                    .execute_blocking(cli)
            }
        }

        let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
            .connect()
            .await
            .unwrap();

        tokio::spawn(drv.into_future());

        let str = String::from("SELECT 1");

        let lifetimed = ExecuteCaptureClient(str.as_str()).execute(&cli).await.unwrap();

        assert_eq!(lifetimed, 1);
    }
}
