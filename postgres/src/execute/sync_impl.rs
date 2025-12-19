use crate::{
    driver::codec::AsParams,
    error::Error,
    prepare::Prepare,
    query::{Query, RowAffected, RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{
        Statement, StatementCreateBlocking, StatementGuarded, StatementNamed, StatementPreparedQuery, StatementQuery,
        StatementSingleRTTQuery,
    },
};

use super::ExecuteBlocking;

impl<'s, C> ExecuteBlocking<&C> for &'s Statement
where
    C: Query,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowStream<'s>, Error>;

    #[inline]
    fn execute_blocking(self, cli: &C) -> Self::ExecuteOutput {
        let stream = self.query_blocking(cli)?;
        RowAffected::from(stream).wait()
    }

    #[inline]
    fn query_blocking(self, cli: &C) -> Self::QueryOutput {
        self.bind_none().query_blocking(cli)
    }
}

impl<C> ExecuteBlocking<&C> for &str
where
    C: Query,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowSimpleStream, Error>;

    #[inline]
    fn execute_blocking(self, cli: &C) -> Self::ExecuteOutput {
        let stream = self.query_blocking(cli)?;
        RowAffected::from(stream).wait()
    }

    #[inline]
    fn query_blocking(self, cli: &C) -> Self::QueryOutput {
        cli._query(self)
    }
}

impl<'c, C> ExecuteBlocking<&'c C> for StatementNamed<'_>
where
    C: Prepare,
{
    type ExecuteOutput = Result<StatementGuarded<'c, C>, Error>;
    type QueryOutput = Self::ExecuteOutput;

    #[inline]
    fn execute_blocking(self, cli: &'c C) -> Self::ExecuteOutput {
        let stmt = cli._query(StatementCreateBlocking::from((self, cli)))??;
        Ok(stmt.into_guarded(cli))
    }

    #[inline]
    fn query_blocking(self, cli: &'c C) -> Self::QueryOutput {
        self.execute_blocking(cli)
    }
}

impl<'s, C, P> ExecuteBlocking<&C> for StatementPreparedQuery<'s, P>
where
    C: Query,
    P: AsParams,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowStream<'s>, Error>;

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        let stream = self.query_blocking(cli)?;
        RowAffected::from(stream).wait()
    }

    #[inline]
    fn query_blocking(self, cli: &C) -> Self::QueryOutput {
        cli._query(self)
    }
}

impl<'c, C, P> ExecuteBlocking<&'c C> for StatementQuery<'_, P>
where
    C: Prepare,
    P: AsParams,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowStreamGuarded<'c, C>, Error>;

    #[inline]
    fn execute_blocking(self, cli: &C) -> Result<u64, Error> {
        self.into_single_rtt().execute_blocking(cli)
    }

    #[inline]
    fn query_blocking(self, cli: &'c C) -> Self::QueryOutput {
        self.into_single_rtt().query_blocking(cli)
    }
}

impl<'c, C, P> ExecuteBlocking<&'c C> for StatementSingleRTTQuery<'_, P>
where
    C: Prepare,
    P: AsParams,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowStreamGuarded<'c, C>, Error>;

    #[inline]
    fn execute_blocking(self, cli: &C) -> Self::ExecuteOutput {
        let stream = self.query_blocking(cli)?;
        RowAffected::from(stream).wait()
    }

    #[inline]
    fn query_blocking(self, cli: &'c C) -> Self::QueryOutput {
        cli._query(self.into_with_cli(cli))
    }
}

impl<C> ExecuteBlocking<&C> for &std::path::Path
where
    C: Query,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowSimpleStream, Error>;

    #[inline]
    fn execute_blocking(self, cli: &C) -> Self::ExecuteOutput {
        std::fs::read_to_string(self)?.execute_blocking(cli)
    }

    #[inline]
    fn query_blocking(self, cli: &C) -> Self::QueryOutput {
        std::fs::read_to_string(self)?.query_blocking(cli)
    }
}
