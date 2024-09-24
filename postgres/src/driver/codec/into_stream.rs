use crate::{
    column::Column,
    prepare::Prepare,
    query::{RowSimpleStream, RowStream, RowStreamGuarded},
};

use super::{sealed, Response};

/// trait for generic over how to construct an async stream rows
pub trait IntoStream: sealed::Sealed + Sized {
    type RowStream<'r>
    where
        Self: 'r;

    fn into_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r;
}

impl sealed::Sealed for &[Column] {}

impl IntoStream for &[Column] {
    type RowStream<'r>
        = RowStream<'r>
    where
        Self: 'r;

    #[inline]
    fn into_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        RowStream::new(res, self)
    }
}

impl sealed::Sealed for Vec<Column> {}

impl IntoStream for Vec<Column> {
    type RowStream<'r>
        = RowSimpleStream
    where
        Self: 'r;

    #[inline]
    fn into_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        RowSimpleStream::new(res, self)
    }
}

pub struct IntoRowStreamGuard<'a, C>(pub &'a C);

impl<C> sealed::Sealed for IntoRowStreamGuard<'_, C> {}

impl<C> IntoStream for IntoRowStreamGuard<'_, C>
where
    C: Prepare,
{
    type RowStream<'r>
        = RowStreamGuarded<'r, C>
    where
        Self: 'r;

    #[inline]
    fn into_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        RowStreamGuarded::new(res, self.0)
    }
}

/// type for case where no row stream can be created.
/// the api caller should never call into_stream method from this type.
pub struct NoOpIntoRowStream;

impl sealed::Sealed for NoOpIntoRowStream {}

impl IntoStream for NoOpIntoRowStream {
    type RowStream<'r>
        = RowStream<'r>
    where
        Self: 'r;

    fn into_stream<'r>(self, _: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        unreachable!("no row stream can be generated from no op row stream constructor")
    }
}
