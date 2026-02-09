use core::{
    future::{Future, Ready, ready},
    pin::Pin,
    task::{Context, Poll},
};

use crate::{
    BoxedFuture,
    client::{Client, ClientBorrow},
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
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowStream<'s>, Error>>;

    #[inline]
    fn execute(self, cli: &Client) -> Self::ExecuteOutput {
        self.bind_none().execute(cli)
    }

    #[inline]
    fn query(self, cli: &Client) -> Self::QueryOutput {
        self.bind_none().query(cli)
    }
}

impl Execute<&Client> for &str {
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowSimpleStream, Error>>;

    #[inline]
    fn execute(self, cli: &Client) -> Self::ExecuteOutput {
        cli.query(self).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &Client) -> Self::QueryOutput {
        ready(cli.query(self))
    }
}

type IntoGuardedFuture<'c, C> = IntoGuarded<'c, BoxedFuture<'c, Result<Statement, Error>>, C>;

pub struct IntoGuarded<'a, F, C> {
    fut: F,
    cli: &'a C,
}

impl<'a, F, C> Future for IntoGuarded<'a, F, C>
where
    C: ClientBorrow,
    F: Future<Output = Result<Statement, Error>> + Unpin,
{
    type Output = Result<StatementGuarded<'a, C>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut this.fut)
            .poll(cx)
            .map_ok(|stmt| stmt.into_guarded(this.cli))
    }
}

impl<'c> Execute<&'c Client> for StatementNamed<'_> {
    type ExecuteOutput = ResultFuture<IntoGuardedFuture<'c, Client>>;
    type QueryOutput = Self::ExecuteOutput;

    #[inline]
    fn execute(self, cli: &'c Client) -> Self::ExecuteOutput {
        cli.query(StatementCreate::from((self, cli)))
            .map(|fut| IntoGuarded { fut, cli })
            .into()
    }

    #[inline]
    fn query(self, cli: &'c Client) -> Self::QueryOutput {
        self.execute(cli)
    }
}

impl<'s, P> Execute<&Client> for StatementPreparedQuery<'s, P>
where
    P: AsParams,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowStream<'s>, Error>>;

    #[inline]
    fn execute(self, cli: &Client) -> Self::ExecuteOutput {
        cli.query(self).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &Client) -> Self::QueryOutput {
        ready(cli.query(self))
    }
}

impl<'s, P> Execute<&Client> for StatementPreparedQueryOwned<'s, P>
where
    P: AsParams,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowStreamOwned, Error>>;

    #[inline]
    fn execute(self, cli: &Client) -> Self::ExecuteOutput {
        cli.query(self).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &Client) -> Self::QueryOutput {
        ready(cli.query(self))
    }
}

impl<'c, P> Execute<&'c Client> for StatementQuery<'_, P>
where
    P: AsParams,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowStreamGuarded<'c, Client>, Error>>;

    #[inline]
    fn execute(self, cli: &Client) -> Self::ExecuteOutput {
        self.into_single_rtt().execute(cli)
    }

    #[inline]
    fn query(self, cli: &'c Client) -> Self::QueryOutput {
        self.into_single_rtt().query(cli)
    }
}

impl<'c, P> Execute<&'c Client> for StatementSingleRTTQuery<'_, P>
where
    P: AsParams,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowStreamGuarded<'c, Client>, Error>>;

    #[inline]
    fn execute(self, cli: &Client) -> Self::ExecuteOutput {
        cli.query(self.into_with_cli(cli)).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &'c Client) -> Self::QueryOutput {
        ready(cli.query(self.into_with_cli(cli)))
    }
}

#[cfg(not(feature = "nightly"))]
impl<'c> Execute<&'c Client> for &std::path::Path {
    type ExecuteOutput = BoxedFuture<'c, Result<u64, Error>>;
    type QueryOutput = BoxedFuture<'c, Result<RowSimpleStream, Error>>;

    #[inline]
    fn execute(self, cli: &'c Client) -> Self::ExecuteOutput {
        let path = self.to_path_buf();
        Box::pin(async move {
            tokio::task::spawn_blocking(|| std::fs::read_to_string(path))
                .await
                .unwrap()?
                .execute(cli)
                .await
        })
    }

    #[inline]
    fn query(self, cli: &'c Client) -> Self::QueryOutput {
        let path = self.to_path_buf();
        Box::pin(async move {
            tokio::task::spawn_blocking(|| std::fs::read_to_string(path))
                .await
                .unwrap()?
                .query(cli)
                .await
        })
    }
}

#[cfg(feature = "nightly")]
impl<'c> Execute<&'c Client> for &std::path::Path {
    type ExecuteOutput = impl Future<Output = Result<u64, Error>> + Send + 'c;
    type QueryOutput = impl Future<Output = Result<RowSimpleStream, Error>> + Send + 'c;

    #[inline]
    fn execute(self, cli: &'c Client) -> Self::ExecuteOutput {
        let path = self.to_path_buf();
        async move {
            tokio::task::spawn_blocking(|| std::fs::read_to_string(path))
                .await
                .unwrap()?
                .execute(cli)
                .await
        }
    }

    #[inline]
    fn query(self, cli: &'c Client) -> Self::QueryOutput {
        let path = self.to_path_buf();
        async move {
            tokio::task::spawn_blocking(|| std::fs::read_to_string(path))
                .await
                .unwrap()?
                .query(cli)
                .await
        }
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
