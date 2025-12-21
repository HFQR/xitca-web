use core::future::Future;

/// async streaming iterator with borrowed Item from Self.
pub trait AsyncLendingIterator {
    type Ok<'i>
    where
        Self: 'i;
    type Err;

    fn try_next(&mut self) -> impl Future<Output = Result<Option<Self::Ok<'_>>, Self::Err>> + Send;

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

impl<I> AsyncLendingIteratorExt for I where I: AsyncLendingIterator {}

pub trait AsyncLendingIteratorExt: AsyncLendingIterator {
    fn map_ok<F, O>(self, func: F) -> MapOk<Self, F>
    where
        F: Fn(Self::Ok<'_>) -> O,
        Self: Sized,
    {
        MapOk { iter: self, func }
    }

    fn map_err<F, O>(self, func: F) -> MapErr<Self, F>
    where
        F: Fn(Self::Err) -> O,
        Self: Sized,
    {
        MapErr { iter: self, func }
    }

    fn try_map<F, T, E>(self, func: F) -> Map<Self, F>
    where
        F: Fn(Result<Self::Ok<'_>, Self::Err>) -> Result<T, E>,
        Self: Sized,
    {
        Map { iter: self, func }
    }

    #[inline]
    fn try_collect<T>(self) -> impl Future<Output = Result<T, Self::Err>> + Send
    where
        T: Default + for<'i> Extend<Self::Ok<'i>> + Send,
        Self: Send + Sized,
    {
        async {
            let mut collection = T::default();
            self.try_collect_into(&mut collection).await?;
            Ok(collection)
        }
    }

    fn try_collect_into<T>(mut self, collection: &mut T) -> impl Future<Output = Result<&mut T, Self::Err>> + Send
    where
        T: for<'i> Extend<Self::Ok<'i>> + Send,
        Self: Send + Sized,
    {
        async move {
            while let Some(item) = self.try_next().await? {
                collection.extend(Some(item));
            }
            Ok(collection)
        }
    }
}

pub struct MapOk<I, F> {
    iter: I,
    func: F,
}

impl<I, F, O> AsyncLendingIterator for MapOk<I, F>
where
    I: AsyncLendingIterator + Send,
    F: Fn(I::Ok<'_>) -> O + Send,
    O: Send,
{
    type Ok<'i>
        = O
    where
        Self: 'i;
    type Err = I::Err;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        let next = self.iter.try_next().await?;
        Ok(next.map(|item| (self.func)(item)))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

pub struct MapErr<I, F> {
    iter: I,
    func: F,
}

impl<I, F, O> AsyncLendingIterator for MapErr<I, F>
where
    I: AsyncLendingIterator + Send,
    F: Fn(I::Err) -> O + Send,
    O: Send,
{
    type Ok<'i>
        = I::Ok<'i>
    where
        Self: 'i;
    type Err = O;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        self.iter.try_next().await.map_err(&self.func)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

pub struct Map<I, F> {
    iter: I,
    func: F,
}

impl<I, F, T, E> AsyncLendingIterator for Map<I, F>
where
    I: AsyncLendingIterator + Send,
    F: Fn(Result<I::Ok<'_>, I::Err>) -> Result<T, E> + Send,
    T: Send,
    E: Send,
{
    type Ok<'i>
        = T
    where
        Self: 'i;
    type Err = E;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        match self.iter.try_next().await {
            Ok(Some(t)) => (self.func)(Ok(t)).map(Some),
            Ok(None) => Ok(None),
            Err(e) => (self.func)(Err(e)).map(Some),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

async fn _map_ok_err_try_collect(stream: crate::RowStreamOwned) -> Result<Vec<String>, crate::error::Error> {
    stream
        .map_ok(|row| row.get(0))
        .map_err(|e| dbg!(e))
        .try_collect::<Vec<_>>()
        .await
}

async fn _connect_quictry_collect(stream: crate::RowStreamOwned) -> Result<Vec<String>, crate::error::Error> {
    stream.try_map(|row| row?.try_get(0)).try_collect::<Vec<_>>().await
}

async fn _try_collect_into(stream: crate::RowStreamOwned) {
    stream
        .try_map(|row| row?.try_get::<i32>(0))
        .try_collect_into(&mut Vec::new())
        .await
        .unwrap()
        .iter()
        .next();
}
