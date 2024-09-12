mod base;
mod row_stream;
mod simple;

pub(crate) mod decode;
pub(crate) mod encode;

pub use base::{Query, RowStream};
pub use simple::{QuerySimple, RowSimpleStream};

use super::BorrowToSql;

/// super trait to constraint Self and associated types' trait bounds.
pub trait AsParams: IntoIterator<IntoIter: ExactSizeIterator, Item: BorrowToSql> {}

impl<I> AsParams for I
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: BorrowToSql,
{
}
