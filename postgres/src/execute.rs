use super::{
    driver::codec::{encode::Encode, into_stream::IntoStream, AsParams},
    error::Error,
    prepare::Prepare,
    query::{ExecuteFuture, Query, RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{Statement, StatementGuarded, StatementQuery, StatementUnnamedBind, StatementUnnamedQuery},
};

/// trait defining how a query is executed.
pub trait Execute<C>
where
    C: Query,
    Self: Sized,
{
    type Encode<'e>: Encode<Output<'e>: IntoStream<RowStream<'e> = Self::RowStream<'e>>>
    where
        C: 'e,
        Self: 'e;
    type RowStream<'r>
    where
        C: 'r,
        Self: 'r;

    /// define how a executable query is constructed.
    ///
    /// this method allows query to reference client type and in some cases the client reference has
    /// to be passed on to returned [`Encode`] type    
    fn into_encode<'c>(self, cli: &'c C) -> Self::Encode<'c>
    where
        Self: 'c;

    /// define how a query is executed with async outcome of how many rows has been affected
    #[inline]
    fn execute(self, cli: &C) -> ExecuteFuture {
        cli._execute(self.into_encode(cli))
    }

    /// define how a query is executed is blocking manner.
    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        self.execute(cli).wait()
    }

    /// define how a query is executed with async streaming of database rows as return type
    #[inline]
    fn query(self, cli: &C) -> Result<Self::RowStream<'_>, Error> {
        cli._query(self.into_encode(cli))
    }
}

impl<C> Execute<C> for &String
where
    C: Query,
{
    type Encode<'e>
        = &'e str
    where
        Self: 'e,
        C: 'e;
    type RowStream<'r>
        = RowSimpleStream
    where
        C: 'r,
        Self: 'r;

    #[inline(always)]
    fn into_encode<'c>(self, _: &'c C) -> Self::Encode<'c>
    where
        Self: 'c,
    {
        self
    }
}

impl<C> Execute<C> for &str
where
    C: Query,
{
    type Encode<'e>
        = &'e str
    where
        Self: 'e,
        C: 'e;
    type RowStream<'r>
        = RowSimpleStream
    where
        C: 'r,
        Self: 'r;

    #[inline(always)]
    fn into_encode<'c>(self, _: &'c C) -> Self::Encode<'c>
    where
        Self: 'c,
    {
        self
    }
}

impl<C, P> Execute<C> for StatementQuery<'_, P>
where
    C: Query,
    P: AsParams,
{
    type Encode<'e>
        = StatementQuery<'e, P>
    where
        Self: 'e,
        C: 'e;
    type RowStream<'r>
        = RowStream<'r>
    where
        C: 'r,
        Self: 'r;

    #[inline(always)]
    fn into_encode<'c>(self, _: &'c C) -> Self::Encode<'c>
    where
        Self: 'c,
    {
        self
    }
}

impl<C> Execute<C> for &Statement
where
    C: Query,
{
    type Encode<'e>
        = StatementQuery<'e, [i32; 0]>
    where
        Self: 'e,
        C: 'e;
    type RowStream<'r>
        = RowStream<'r>
    where
        C: 'r,
        Self: 'r;

    #[inline]
    fn into_encode<'c>(self, _: &'c C) -> Self::Encode<'c>
    where
        Self: 'c,
    {
        StatementQuery { stmt: self, params: [] }
    }
}

impl<C> Execute<C> for &StatementGuarded<'_, C>
where
    C: Prepare,
{
    type Encode<'e>
        = StatementQuery<'e, [i32; 0]>
    where
        Self: 'e,
        C: 'e;
    type RowStream<'r>
        = RowStream<'r>
    where
        C: 'r,
        Self: 'r;

    #[inline]
    fn into_encode<'c>(self, _: &'c C) -> Self::Encode<'c>
    where
        Self: 'c,
    {
        StatementQuery { stmt: self, params: [] }
    }
}

impl<'a, C, P> Execute<C> for StatementUnnamedBind<'a, P>
where
    C: Query + Prepare,
    P: AsParams,
{
    type Encode<'e>
        = StatementUnnamedQuery<'e, P, C>
    where
        Self: 'e,
        C: 'e;
    type RowStream<'r>
        = RowStreamGuarded<'r, C>
    where
        C: 'r,
        Self: 'r;

    #[inline]
    fn into_encode<'c>(self, cli: &'c C) -> Self::Encode<'c>
    where
        Self: 'c,
    {
        StatementUnnamedQuery {
            stmt: self.stmt.stmt,
            types: self.stmt.types,
            params: self.params,
            cli,
        }
    }
}
