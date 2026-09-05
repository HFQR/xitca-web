use std::sync::Arc;

use crate::{
    driver::codec::AsParams,
    error::Error,
    execute::Execute,
    query::{RowAffected, RowSimpleStream, RowStreamOwned},
    statement::StatementQuery,
};

use super::{Pool, PoolOwned};

impl<'c, 's> Execute<&'c Pool> for &'s str
where
    's: 'c,
{
    type ExecuteOutput = u64;
    type QueryOutput = RowSimpleStream;

    #[inline]
    async fn execute(self, pool: &'c Pool) -> Result<Self::ExecuteOutput, Error> {
        // response is owned so the connection goes back to the pool before it's awaited.
        let affected = {
            let conn = pool.get().await?;
            self.query(&conn).await.map(RowAffected::from)
        };
        affected?.await
    }

    #[inline]
    async fn query(self, pool: &'c Pool) -> Result<Self::QueryOutput, Error> {
        let conn = pool.get().await?;
        self.query(&conn).await
    }
}

impl<'c, 's, P> Execute<&'c Pool> for StatementQuery<'s, P>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = u64;
    type QueryOutput = RowStreamOwned;

    #[inline]
    async fn execute(self, pool: &'c Pool) -> Result<Self::ExecuteOutput, Error> {
        {
            let mut conn = pool.get().await?;
            self.execute(&mut conn).await?
        }
        // return connection to pool before await on execution future
        .await
    }

    #[inline]
    async fn query(self, pool: &'c Pool) -> Result<Self::QueryOutput, Error> {
        let mut conn = pool.get().await?;
        self.query(&mut conn).await
    }
}

impl<'c, 's, P, const N: usize> Execute<&'c Pool> for [StatementQuery<'s, P>; N]
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = u64;
    type QueryOutput = Vec<RowStreamOwned>;

    #[inline]
    fn execute(self, pool: &'c Pool) -> impl Future<Output = Result<Self::ExecuteOutput, Error>> + Send {
        execute_iter_with_pool(self.into_iter(), pool)
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        query_iter_with_pool(self.into_iter(), pool)
    }
}

impl<'c, 's, P> Execute<&'c Pool> for Vec<StatementQuery<'s, P>>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = u64;
    type QueryOutput = Vec<RowStreamOwned>;

    #[inline]
    fn execute(self, pool: &'c Pool) -> impl Future<Output = Result<Self::ExecuteOutput, Error>> + Send {
        execute_iter_with_pool(self.into_iter(), pool)
    }

    #[inline]
    fn query(self, pool: &'c Pool) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
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
    fn execute(self, pool: &'c Arc<Pool>) -> impl Future<Output = Result<Self::ExecuteOutput, Error>> + Send {
        Q::execute(self, pool)
    }

    #[inline]
    fn query(self, pool: &'c Arc<Pool>) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        Q::query(self, pool)
    }
}

impl<'c, 's> Execute<&'c PoolOwned> for &'s str
where
    's: 'c,
{
    type ExecuteOutput = u64;
    type QueryOutput = RowSimpleStream;

    #[inline]
    async fn execute(self, pool: &'c PoolOwned) -> Result<Self::ExecuteOutput, Error> {
        // response is owned so the connection goes back to the pool before it's awaited.
        let affected = {
            let conn = pool.get().await?;
            self.query(&conn).await.map(RowAffected::from)
        };
        affected?.await
    }

    #[inline]
    async fn query(self, pool: &'c PoolOwned) -> Result<Self::QueryOutput, Error> {
        let conn = pool.get().await?;
        self.query(&conn).await
    }
}

impl<'c, 's, P> Execute<&'c PoolOwned> for StatementQuery<'s, P>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = u64;
    type QueryOutput = RowStreamOwned;

    #[inline]
    async fn execute(self, pool: &'c PoolOwned) -> Result<Self::ExecuteOutput, Error> {
        {
            let mut conn = pool.get().await?;
            self.execute(&mut conn).await?
        }
        // return connection to pool before await on execution future
        .await
    }

    #[inline]
    async fn query(self, pool: &'c PoolOwned) -> Result<Self::QueryOutput, Error> {
        let mut conn = pool.get().await?;
        self.query(&mut conn).await
    }
}

impl<'c, 's, P, const N: usize> Execute<&'c PoolOwned> for [StatementQuery<'s, P>; N]
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = u64;
    type QueryOutput = Vec<RowStreamOwned>;

    #[inline]
    fn execute(self, pool: &'c PoolOwned) -> impl Future<Output = Result<Self::ExecuteOutput, Error>> + Send {
        execute_iter_with_pool_owned(self.into_iter(), pool)
    }

    #[inline]
    fn query(self, pool: &'c PoolOwned) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        query_iter_with_pool_owned(self.into_iter(), pool)
    }
}

impl<'c, 's, P> Execute<&'c PoolOwned> for Vec<StatementQuery<'s, P>>
where
    P: AsParams + Send + 'c,
    's: 'c,
{
    type ExecuteOutput = u64;
    type QueryOutput = Vec<RowStreamOwned>;

    #[inline]
    fn execute(self, pool: &'c PoolOwned) -> impl Future<Output = Result<Self::ExecuteOutput, Error>> + Send {
        execute_iter_with_pool_owned(self.into_iter(), pool)
    }

    #[inline]
    fn query(self, pool: &'c PoolOwned) -> impl Future<Output = Result<Self::QueryOutput, Error>> + Send {
        query_iter_with_pool_owned(self.into_iter(), pool)
    }
}

async fn execute_iter_with_pool_owned<P>(
    iter: impl Iterator<Item = StatementQuery<'_, P>> + Send,
    pool: &PoolOwned,
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

async fn query_iter_with_pool_owned<P>(
    iter: impl Iterator<Item = StatementQuery<'_, P>> + Send,
    pool: &PoolOwned,
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
