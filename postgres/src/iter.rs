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
