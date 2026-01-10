use std::sync::Arc;

use crate::{
    driver::codec::AsParams,
    error::Error,
    execute::Execute,
    query::{RowSimpleStream, RowStreamOwned},
    statement::StatementQuery,
};

use super::Pool;

#[cfg(not(feature = "nightly"))]
impl<'c, 's> Execute<&'c Pool> for &'s str
where
    's: 'c,
{
    type ExecuteOutput = crate::BoxedFuture<'c, Result<u64, Error>>;
    type QueryOutput = crate::BoxedFuture<'c, Result<RowSimpleStream, Error>>;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        Box::pin(async {
            {
                let conn = pool.get().await?;
                self.execute(&conn)
            }
            // return connection to pool before await on execution future
            .await
        })
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        Box::pin(async {
            let conn = pool.get().await?;
            self.query(&conn).await
        })
    }
}

#[cfg(feature = "nightly")]
impl<'c, 's> Execute<&'c Pool> for &'s str
where
    's: 'c,
{
    type ExecuteOutput = impl Future<Output = Result<u64, Error>> + Send + 'c;
    type QueryOutput = impl Future<Output = Result<RowSimpleStream, Error>> + Send + 'c;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        async {
            {
                let conn = pool.get().await?;
                self.execute(&conn)
            }
            // return connection to pool before await on execution future
            .await
        }
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        async {
            let conn = pool.get().await?;
            self.query(&conn).await
        }
    }
}

#[cfg(not(feature = "nightly"))]
impl<'c, 's, P> Execute<&'c Pool> for StatementQuery<'s, P>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = crate::BoxedFuture<'c, Result<u64, Error>>;
    type QueryOutput = crate::BoxedFuture<'c, Result<RowStreamOwned, Error>>;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        Box::pin(async {
            {
                let mut conn = pool.get().await?;
                self.execute(&mut conn).await?
            }
            // return connection to pool before await on execution future
            .await
        })
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        Box::pin(async {
            let mut conn = pool.get().await?;
            self.query(&mut conn).await
        })
    }
}

#[cfg(feature = "nightly")]
impl<'c, 's, P> Execute<&'c Pool> for StatementQuery<'s, P>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = impl Future<Output = Result<u64, Error>> + Send + 'c;
    type QueryOutput = impl Future<Output = Result<RowStreamOwned, Error>> + Send + 'c;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        async {
            {
                let mut conn = pool.get().await?;
                self.execute(&mut conn).await?
            }
            // return connection to pool before await on execution future
            .await
        }
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        async {
            let mut conn = pool.get().await?;
            self.query(&mut conn).await
        }
    }
}

#[cfg(not(feature = "nightly"))]
impl<'c, 's, P, const N: usize> Execute<&'c Pool> for [StatementQuery<'s, P>; N]
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = crate::BoxedFuture<'c, Result<u64, Error>>;
    type QueryOutput = crate::BoxedFuture<'c, Result<Vec<RowStreamOwned>, Error>>;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        Box::pin(execute_iter_with_pool(self.into_iter(), pool))
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        Box::pin(query_iter_with_pool(self.into_iter(), pool))
    }
}

#[cfg(feature = "nightly")]
impl<'c, 's, P, const N: usize> Execute<&'c Pool> for [StatementQuery<'s, P>; N]
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = impl Future<Output = Result<u64, Error>> + Send + 'c;
    type QueryOutput = impl Future<Output = Result<Vec<RowStreamOwned>, Error>> + Send + 'c;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        execute_iter_with_pool(self.into_iter(), pool)
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        query_iter_with_pool(self.into_iter(), pool)
    }
}

#[cfg(not(feature = "nightly"))]
impl<'c, 's, P> Execute<&'c Pool> for Vec<StatementQuery<'s, P>>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = crate::BoxedFuture<'c, Result<u64, Error>>;
    type QueryOutput = crate::BoxedFuture<'c, Result<Vec<RowStreamOwned>, Error>>;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        Box::pin(execute_iter_with_pool(self.into_iter(), pool))
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        Box::pin(query_iter_with_pool(self.into_iter(), pool))
    }
}

#[cfg(feature = "nightly")]
impl<'c, 's, P> Execute<&'c Pool> for Vec<StatementQuery<'s, P>>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = impl Future<Output = Result<u64, Error>> + Send + 'c;
    type QueryOutput = impl Future<Output = Result<Vec<RowStreamOwned>, Error>> + Send + 'c;

    #[inline]
    fn execute(self, pool: &'c Pool) -> Self::ExecuteOutput {
        execute_iter_with_pool(self.into_iter(), pool)
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> Self::QueryOutput {
        query_iter_with_pool(self.into_iter(), pool)
    }
}

async fn execute_iter_with_pool<P>(
    iter: impl Iterator<Item = StatementQuery<'_, P>> + Send,
    pool: &Pool,
) -> Result<u64, Error>
where
    P: AsParams + Send,
{
    let mut res = Vec::with_capacity(iter.size_hint().0);

    {
        let mut conn = pool.get().await?;

        for stmt in iter {
            let fut = stmt.execute(&mut conn).await?;
            res.push(fut);
        }
    }

    let mut num = 0;

    for res in res {
        num += res.await?;
    }

    Ok(num)
}

async fn query_iter_with_pool<P>(
    iter: impl Iterator<Item = StatementQuery<'_, P>> + Send,
    pool: &Pool,
) -> Result<Vec<RowStreamOwned>, Error>
where
    P: AsParams + Send,
{
    let mut res = Vec::with_capacity(iter.size_hint().0);

    let mut conn = pool.get().await?;

    for stmt in iter {
        let stream = stmt.query(&mut conn).await?;
        res.push(stream);
    }

    Ok(res)
}

impl<'c, Q> Execute<&'c Arc<Pool>> for Q
where
    Q: Execute<&'c Pool>,
{
    type ExecuteOutput = Q::ExecuteOutput;
    type QueryOutput = Q::QueryOutput;

    #[inline]
    fn execute(self, pool: &'c Arc<Pool>) -> Self::ExecuteOutput {
        Q::execute(self, pool)
    }

    #[inline]
    fn query(self, pool: &'c Arc<Pool>) -> Self::QueryOutput {
        Q::query(self, pool)
    }
}
