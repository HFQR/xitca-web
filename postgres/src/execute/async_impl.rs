use core::future::{ready, Future, Ready};

use crate::{
    driver::codec::AsParams,
    error::Error,
    prepare::Prepare,
    query::{Query, RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{
        Statement, StatementCreate, StatementGuarded, StatementNamed, StatementQuery, StatementUnnamedBind,
        StatementUnnamedQuery,
    },
};

use super::Execute;

impl<'s, C> Execute<&C> for &'s Statement
where
    C: Query,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowStream<'s>, Error>;

    #[inline]
    fn execute(self, cli: &C) -> impl Future<Output = Self::ExecuteOutput> + 'static {
        let res = cli._query(self).map(|res| res.res);
        async { res?.try_into_row_affected().await }
    }

    #[inline]
    fn query(self, cli: &C) -> Ready<Self::QueryOutput> {
        ready(cli._query(self))
    }
}

impl<C> Execute<&C> for &str
where
    C: Query,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowSimpleStream, Error>;

    #[inline]
    fn execute(self, cli: &C) -> impl Future<Output = Self::ExecuteOutput> + 'static {
        let res = cli._query(self).map(|res| res.res);
        async { res?.try_into_row_affected().await }
    }

    #[inline]
    fn query(self, cli: &C) -> Ready<Self::QueryOutput> {
        ready(cli._query(self))
    }
}

impl<'c, C> Execute<&'c C> for StatementNamed<'_>
where
    C: Prepare,
{
    type ExecuteOutput = Result<StatementGuarded<'c, C>, Error>;
    type QueryOutput = Self::ExecuteOutput;

    #[inline]
    async fn execute(self, cli: &'c C) -> Self::ExecuteOutput {
        cli._query(StatementCreate::from((self, cli)))?
            .await
            .map(|stmt| stmt.into_guarded(cli))
    }

    #[inline]
    fn query(self, cli: &'c C) -> impl Future<Output = Self::QueryOutput> {
        self.execute(cli)
    }
}

impl<'s, C, P> Execute<&C> for StatementQuery<'s, P>
where
    C: Query,
    P: AsParams,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowStream<'s>, Error>;

    #[inline]
    fn execute(self, cli: &C) -> impl Future<Output = Self::ExecuteOutput> + 'static {
        let res = cli._query(self).map(|res| res.res);
        async { res?.try_into_row_affected().await }
    }

    #[inline]
    fn query(self, cli: &C) -> Ready<Self::QueryOutput> {
        ready(cli._query(self))
    }
}

impl<'c, C, P> Execute<&'c C> for StatementUnnamedBind<'_, P>
where
    C: Prepare,
    P: AsParams,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowStreamGuarded<'c, C>, Error>;

    #[inline]
    async fn execute(self, cli: &C) -> Self::ExecuteOutput {
        cli._query(StatementUnnamedQuery::from((self, cli)))?
            .res
            .try_into_row_affected()
            .await
    }

    #[inline]
    fn query(self, cli: &'c C) -> Ready<Self::QueryOutput> {
        ready(cli._query(StatementUnnamedQuery::from((self, cli))))
    }
}

impl<'c, C> Execute<&'c C> for &std::path::Path
where
    C: Query + Sync,
{
    type ExecuteOutput = Result<u64, Error>;
    type QueryOutput = Result<RowSimpleStream, Error>;

    #[inline]
    async fn execute(self, cli: &'c C) -> Self::ExecuteOutput {
        let path = self.to_path_buf();
        tokio::task::spawn_blocking(|| std::fs::read_to_string(path))
            .await
            .unwrap()?
            .execute(cli)
            .await
    }

    #[inline]
    async fn query(self, cli: &'c C) -> Self::QueryOutput {
        let path = self.to_path_buf();
        tokio::task::spawn_blocking(|| std::fs::read_to_string(path))
            .await
            .unwrap()?
            .query(cli)
            .await
    }
}
