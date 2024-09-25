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

pub trait AsyncLendingIteratorExt: AsyncLendingIterator {
    fn map_ok<F, O>(self, func: F) -> MapOk<Self, F>
    where
        F: Fn(Self::Ok<'_>) -> O,
        Self: Sized,
    {
        MapOk { iter: self, func }
    }

    #[inline]
    fn try_collect<T>(self) -> impl Future<Output = Result<T, Self::Err>> + Send
    where
        T: Default + for<'i> Extend<Self::Ok<'i>> + Send,
        Self: Send + Sized,
    {
        self.try_collect_into(T::default())
    }

    fn try_collect_into<T>(mut self, mut collection: T) -> impl Future<Output = Result<T, Self::Err>> + Send
    where
        T: for<'i> Extend<Self::Ok<'i>> + Send,
        Self: Send + Sized,
    {
        async move {
            while let Some(item) = self.try_next().await? {
                collection.extend([item]);
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

impl<I> AsyncLendingIteratorExt for I where I: AsyncLendingIterator {}

async fn _try_collect_test(stream: crate::RowStreamOwned) -> Result<Vec<String>, crate::error::Error> {
    stream.map_ok(|row| row.get(0)).try_collect::<Vec<_>>().await
}
