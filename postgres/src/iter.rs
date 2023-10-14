use core::future::Future;

use super::ToSql;

/// async streaming iterator with borrowed Item from Self.
pub trait AsyncIterator {
    type Item<'i>
    where
        Self: 'i;

    fn next(&mut self) -> impl Future<Output = Option<Self::Item<'_>>> + Send;

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

pub(super) fn slice_iter<'a>(s: &'a [&(dyn ToSql + Sync)]) -> impl ExactSizeIterator<Item = &'a dyn ToSql> {
    s.iter().map(|s| *s as _)
}
