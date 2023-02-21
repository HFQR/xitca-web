use core::{
    pin::Pin,
    task::{Context, Poll},
};

use super::ToSql;

/// async streaming iterator with borrowed Item from Self.
pub trait AsyncIterator {
    type Item<'i>
    where
        Self: 'i;

    fn poll_next<'s>(self: Pin<&'s mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item<'s>>>
    where
        Self: 's;

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

pub(super) fn slice_iter<'a>(s: &'a [&(dyn ToSql + Sync)]) -> impl ExactSizeIterator<Item = &'a dyn ToSql> {
    s.iter().map(|s| *s as _)
}
