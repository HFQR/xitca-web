use crate::{
    client::Client,
    driver::codec::AsParams,
    error::Error,
    query::{RowAffected, RowSimpleStream, RowStream, RowStreamGuarded, RowStreamOwned},
    statement::{
        Statement, StatementCreate, StatementGuarded, StatementNamed, StatementPreparedQuery,
        StatementPreparedQueryOwned, StatementQuery, StatementSingleRTTQuery,
    },
};

use super::Execute;

impl<'s> Execute<&Client> for &'s Statement {
    type ExecuteOutput = u64;
    type QueryOutput = RowStream<'s>;

    #[inline]
    fn execute(self, cli: &Client) -> impl Future<Output = Result<Self::ExecuteOutput, Error>> + Send {
        self.bind_none().execute(cli)
    }

    #[inline]
    fn query(self, cli: &Client) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        self.bind_none().query(cli)
    }
}

impl Execute<&Client> for &str {
    type ExecuteOutput = u64;
    type QueryOutput = RowSimpleStream;

    #[inline]
    async fn execute(self, cli: &Client) -> Result<Self::ExecuteOutput, Error> {
        self.query(cli).await.map(RowAffected::from)?.await
    }

    #[inline]
    fn query(self, cli: &Client) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        cli.query(self)
    }
}

impl<'c> Execute<&'c Client> for StatementNamed<'_> {
    type ExecuteOutput = StatementGuarded<'c, Client>;
    type QueryOutput = Self::ExecuteOutput;

    #[inline]
    async fn execute(self, cli: &'c Client) -> Result<Self::ExecuteOutput, Error> {
        cli.query(StatementCreate::from((self, cli)))
            .await?
            .await
            .map(|stmt| stmt.into_guarded(cli))
    }

    #[inline]
    fn query(self, cli: &'c Client) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        self.execute(cli)
    }
}

impl<'s, P> Execute<&Client> for StatementPreparedQuery<'s, P>
where
    P: AsParams + Send,
{
    type ExecuteOutput = u64;
    type QueryOutput = RowStream<'s>;

    #[inline]
    async fn execute(self, cli: &Client) -> Result<Self::ExecuteOutput, Error> {
        self.query(cli).await.map(RowAffected::from)?.await
    }

    #[inline]
    fn query(self, cli: &Client) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        cli.query(self)
    }
}

impl<'s, P> Execute<&Client> for StatementPreparedQueryOwned<'s, P>
where
    P: AsParams + Send,
{
    type ExecuteOutput = u64;
    type QueryOutput = RowStreamOwned;

    #[inline]
    async fn execute(self, cli: &Client) -> Result<Self::ExecuteOutput, Error> {
        self.query(cli).await.map(RowAffected::from)?.await
    }

    #[inline]
    fn query(self, cli: &Client) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        cli.query(self)
    }
}

impl<'c, P> Execute<&'c Client> for StatementQuery<'_, P>
where
    P: AsParams + Send,
{
    type ExecuteOutput = u64;
    type QueryOutput = RowStreamGuarded<'c, Client>;

    #[inline]
    fn execute(self, cli: &'c Client) -> impl Future<Output = Result<Self::ExecuteOutput, Error>> + Send {
        self.into_single_rtt().execute(cli)
    }

    #[inline]
    fn query(self, cli: &'c Client) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        self.into_single_rtt().query(cli)
    }
}

impl<'c, P> Execute<&'c Client> for StatementSingleRTTQuery<'_, P>
where
    P: AsParams + Send,
{
    type ExecuteOutput = u64;
    type QueryOutput = RowStreamGuarded<'c, Client>;

    #[inline]
    async fn execute(self, cli: &'c Client) -> Result<Self::ExecuteOutput, Error> {
        self.query(cli).await.map(RowAffected::from)?.await
    }

    #[inline]
    fn query(self, cli: &'c Client) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        cli.query(self.into_with_cli(cli))
    }
}

impl<'c> Execute<&'c Client> for &std::path::Path {
    type ExecuteOutput = u64;
    type QueryOutput = RowSimpleStream;

    #[inline]
    async fn execute(self, cli: &'c Client) -> Result<Self::ExecuteOutput, Error> {
        read_to_string(self).await?.execute(cli).await
    }

    #[inline]
    async fn query(self, cli: &'c Client) -> Result<Self::QueryOutput, Error> {
        read_to_string(self).await?.query(cli).await
    }
}

async fn read_to_string(path: &std::path::Path) -> Result<String, Error> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(|| std::fs::read_to_string(path))
        .await
        .unwrap()
        .map_err(Into::into)
}
