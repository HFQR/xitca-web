use core::{
    future::{ready, Future, Ready},
    pin::Pin,
    task::{Context, Poll},
};

use crate::{
    driver::codec::AsParams,
    error::Error,
    prepare::Prepare,
    query::{Query, RowAffected, RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{
        Statement, StatementCreate, StatementGuarded, StatementNamed, StatementQuery, StatementUnnamedBind,
        StatementUnnamedQuery,
    },
    BoxedFuture,
};

use super::Execute;

impl<'s, C> Execute<&C> for &'s Statement
where
    C: Query,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowStream<'s>, Error>>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteOutput {
        cli._query(self).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &C) -> Self::QueryOutput {
        ready(cli._query(self))
    }
}

impl<C> Execute<&C> for &str
where
    C: Query,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowSimpleStream, Error>>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteOutput {
        cli._query(self).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &C) -> Self::QueryOutput {
        ready(cli._query(self))
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
    C: Query,
{
    type Output = Result<StatementGuarded<'a, C>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut this.fut)
            .poll(cx)
            .map_ok(|stmt| stmt.into_guarded(this.cli))
    }
}

impl<'c, C> Execute<&'c C> for StatementNamed<'_>
where
    C: Prepare,
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
}

impl<'s, C, P> Execute<&C> for StatementQuery<'s, P>
where
    C: Query,
    P: AsParams,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowStream<'s>, Error>>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteOutput {
        cli._query(self).map(RowAffected::from).into()
    }

    #[inline]
    fn query(self, cli: &C) -> Self::QueryOutput {
        ready(cli._query(self))
    }
}

impl<'c, C, P> Execute<&'c C> for StatementUnnamedBind<'_, P>
where
    C: Prepare,
    P: AsParams,
{
    type ExecuteOutput = ResultFuture<RowAffected>;
    type QueryOutput = Ready<Result<RowStreamGuarded<'c, C>, Error>>;

    #[inline]
    fn execute(self, cli: &C) -> Self::ExecuteOutput {
        cli._query(StatementUnnamedQuery::from((self, cli)))
            .map(RowAffected::from)
            .into()
    }

    #[inline]
    fn query(self, cli: &'c C) -> Self::QueryOutput {
        ready(cli._query(StatementUnnamedQuery::from((self, cli))))
    }
}

impl<'c, C> Execute<&'c C> for &std::path::Path
where
    C: Query + Sync,
{
    type ExecuteOutput = BoxedFuture<'c, Result<u64, Error>>;
    type QueryOutput = BoxedFuture<'c, Result<RowSimpleStream, Error>>;

    #[inline]
    fn execute(self, cli: &'c C) -> Self::ExecuteOutput {
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
    fn query(self, cli: &'c C) -> Self::QueryOutput {
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
