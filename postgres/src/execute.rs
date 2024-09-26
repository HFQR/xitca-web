use super::{
    driver::codec::AsParams,
    error::Error,
    prepare::Prepare,
    query::{ExecuteFuture, Query, RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{Statement, StatementQuery, StatementUnnamedQuery},
};

/// trait defining how a query is executed.
pub trait Execute<C>
where
    C: Query,
    Self: Sized,
{
    type RowStream<'r>
    where
        Self: 'r;

    /// define how a query is executed with async outcome of how many rows has been affected
    fn execute(self, cli: &C) -> ExecuteFuture;

    /// define how a query is executed with async streaming of database rows as return type
    fn query<'r>(self, cli: &C) -> Result<Self::RowStream<'r>, Error>;

    /// define how a query is executed is blocking manner.
    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        self.execute(cli).wait()
    }
}

impl<C> Execute<C> for &Statement
where
    C: Query,
{
    type RowStream<'r>
        = RowStream<'r>
    where
        Self: 'r;

    #[inline]
    fn execute(self, cli: &C) -> ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query<'r>(self, cli: &C) -> Result<Self::RowStream<'r>, Error>
    where
        Self: 'r,
    {
        cli._query(self)
    }
}

impl<C> Execute<C> for &str
where
    C: Query,
{
    type RowStream<'r>
        = RowSimpleStream
    where
        Self: 'r;

    #[inline]
    fn execute(self, cli: &C) -> ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query<'r>(self, cli: &C) -> Result<Self::RowStream<'r>, Error>
    where
        Self: 'r,
    {
        cli._query(self)
    }
}

impl<'a, C, P> Execute<C> for StatementQuery<'a, P>
where
    C: Query + Prepare,
    P: AsParams,
{
    type RowStream<'r>
        = RowStream<'r>
    where
        Self: 'r;

    #[inline]
    fn execute(self, cli: &C) -> ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query<'r>(self, cli: &C) -> Result<Self::RowStream<'r>, Error>
    where
        Self: 'r,
    {
        cli._query(self)
    }
}

impl<'a, C, P> Execute<C> for StatementUnnamedQuery<'a, P, C>
where
    C: Query + Prepare,
    P: AsParams,
{
    type RowStream<'r>
        = RowStreamGuarded<'r, C>
    where
        Self: 'r;

    #[inline]
    fn execute(self, cli: &C) -> ExecuteFuture {
        cli._execute(self)
    }

    #[inline]
    fn query<'r>(self, cli: &C) -> Result<Self::RowStream<'r>, Error>
    where
        Self: 'r,
    {
        cli._query(self)
    }
}
