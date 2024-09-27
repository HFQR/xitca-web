use core::future::Future;

use super::{
    driver::codec::AsParams,
    error::Error,
    prepare::Prepare,
    query::{ExecuteFuture, Query, RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{Statement, StatementQuery, StatementUnnamedBind, StatementUnnamedQuery},
};

/// Defining how a query is executed. can be used for customizing encoding, executing and database
/// data decoding.
///
/// For customized encoding please see [`Encode`] trait for detail.
/// For customized decoding please see [`IntoStream`] trait for detail.
///
/// [`Encode`]: crate::driver::codec::encode::Encode
/// [`IntoStream`]: crate::driver::codec::into_stream::IntoStream
pub trait Execute<'c, C>
where
    C: Query,
    Self: Sized,
{
    type ExecuteFuture: Future<Output = Result<u64, Error>>;
    /// iterator type of database rows.
    ///
    /// by default this type should be matching `C`'s [`Query::_query`] output type.
    ///
    /// consider impl [`AsyncLendingIterator`] for async iterator of rows
    /// consider impl [`Iterator`] for iterator of rows
    ///
    /// [`IntoStream::RowStream`]: crate::driver::codec::into_stream::IntoStream::RowStream
    /// [`AsyncLendingIterator`]: crate::iter::AsyncLendingIterator
    type RowStream;

    /// define how a query is executed with async outcome of how many rows has been affected
    fn execute(self, cli: &'c C) -> Self::ExecuteFuture;

    /// define how a query is executed with streaming of database rows as return type.
    fn query(self, cli: &'c C) -> Result<Self::RowStream, Error>;

    /// blocking version of [`Execute::execute`]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error>;
}

impl<'s, C> Execute<'_, C> for &'s Statement
where
    C: Query,
{
    type ExecuteFuture = ExecuteFuture;
    type RowStream = RowStream<'s>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query(self, cli: &C) -> Result<Self::RowStream, Error> {
        cli._query(self)
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        self.execute(cli).wait()
    }
}

impl<'s, C> Execute<'_, C> for &'s str
where
    C: Query,
{
    type ExecuteFuture = ExecuteFuture;
    type RowStream = RowSimpleStream;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query(self, cli: &C) -> Result<Self::RowStream, Error> {
        cli._query(self)
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        self.execute(cli).wait()
    }
}

impl<'s, C, P> Execute<'_, C> for StatementQuery<'s, P>
where
    C: Query + Prepare,
    P: AsParams + 's,
{
    type ExecuteFuture = ExecuteFuture;
    type RowStream = RowStream<'s>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query(self, cli: &C) -> Result<Self::RowStream, Error> {
        cli._query(self)
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        self.execute(cli).wait()
    }
}

impl<'s, 'c, C, P> Execute<'c, C> for StatementUnnamedBind<'s, P>
where
    C: Query + Prepare + 'c,
    P: AsParams + 's + 'c,
    's: 'c,
{
    type ExecuteFuture = ExecuteFuture;
    type RowStream = RowStreamGuarded<'c, C>;

    #[inline]
    fn execute(self, cli: &'c C) -> Self::ExecuteFuture {
        cli._execute(StatementUnnamedQuery::from((self, cli)))
    }

    #[inline]
    fn query(self, cli: &'c C) -> Result<Self::RowStream, Error> {
        cli._query(StatementUnnamedQuery::from((self, cli)))
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        self.execute(cli).wait()
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
                    let stmt = cli._prepare(self.0, &[]).await?;
                    stmt.execute(cli).await
                })
            }

            fn query(self, _: &'c C) -> Result<Self::RowStream, Error> {
                todo!()
            }

            fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
                let stmt = cli._prepare_blocking(self.0, &[])?;
                stmt.execute_blocking(cli)
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
