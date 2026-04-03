//! [`Stream`] trait impl for types already implemented [`Body`] trait
//!
//! # IMPORTANT
//! [`Stream::size_hint`] does not provide a fixed state for expressing [`SizeHint::None`].
//! Therefore this crate decides to use [`SizeHint::NO_BODY_HINT`] as mapped expression to it.

use core::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Buf;
use futures_core::stream::Stream;

use super::{
    body::Body,
    frame::Frame,
    size_hint::SizeHint,
    util::{Either, Empty, Full, StreamBody},
};

macro_rules! stream_impls {
    ($ty:ident<$($gen:ident $(: $bound:path)?),+>) => {
        impl<$($gen $(: $bound)?),+> Stream for $ty<$($gen),+>
        where
            $ty<$($gen),+>: Body,
        {
            type Item = Result<Frame<<$ty<$($gen),+> as Body>::Data>, <$ty<$($gen),+> as Body>::Error>;

            #[inline]
            fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
                Body::poll_frame(self, cx)
            }

            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                match Body::size_hint(self) {
                    SizeHint::Exact(len) => {
                        let len = usize::try_from(len).unwrap();
                        (len, Some(len))
                    }
                    SizeHint::Unknown => (0, None),
                    SizeHint::None => SizeHint::NO_BODY_HINT,
                }
            }
        }
    };
}

stream_impls!(Full<D: Buf>);
stream_impls!(Empty<D>);
stream_impls!(Either<L, R>);
stream_impls!(StreamBody<S>);
