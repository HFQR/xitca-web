use super::{
    driver::codec::AsParams,
    error::Error,
    prepare::Prepare,
    query::{ExecuteFuture, Query, RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{Statement, StatementQuery, StatementUnnamedBind, StatementUnnamedQuery},
};

/// trait defining how a query is executed.
pub trait Execute<'c, C>
where
    C: Query,
    Self: Sized,
{
    type ExecuteFuture;
    type RowStream;

    /// define how a query is executed with async outcome of how many rows has been affected
    fn execute(self, cli: &'c C) -> Self::ExecuteFuture;

    /// define how a query is executed with async streaming of database rows as return type
    fn query(self, cli: &'c C) -> Result<Self::RowStream, Error>;

    /// blocking version of [`Execute::execute`]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error>;
}

impl<'s, 'c, C> Execute<'c, C> for &'s Statement
where
    C: Query,
{
    type ExecuteFuture = ExecuteFuture;
    type RowStream = RowStream<'s>;

    #[inline]
    fn execute(self, cli: &'c C) -> Self::ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query(self, cli: &'c C) -> Result<Self::RowStream, Error> {
        cli._query(self)
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        self.execute(cli).wait()
    }
}

impl<'s, 'c, C> Execute<'c, C> for &'s str
where
    C: Query,
{
    type ExecuteFuture = ExecuteFuture;
    type RowStream = RowSimpleStream;

    #[inline]
    fn execute(self, cli: &'c C) -> Self::ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query(self, cli: &'c C) -> Result<Self::RowStream, Error> {
        cli._query(self)
    }

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        self.execute(cli).wait()
    }
}

impl<'s, 'c, C, P> Execute<'c, C> for StatementQuery<'s, P>
where
    C: Query + Prepare,
    P: AsParams + 's,
{
    type ExecuteFuture = ExecuteFuture;
    type RowStream = RowStream<'s>;

    #[inline]
    fn execute(self, cli: &'c C) -> Self::ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query(self, cli: &'c C) -> Result<Self::RowStream, Error> {
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
