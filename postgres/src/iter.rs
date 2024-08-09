use core::future::Future;

use super::ToSql;

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

pub(super) fn slice_iter<'a>(s: &'a [&(dyn ToSql + Sync)]) -> impl ExactSizeIterator<Item = &'a dyn ToSql> + Clone {
    s.iter().map(|s| *s as _)
}
