use crate::{column::Column, Type};

mod sealed {
    pub trait Sealed {}
}

use self::sealed::Sealed;

/// a trait implemented by types that can find index and it's associated [Type] into columns of a
/// row. cannot be implemented beyond crate boundary.
pub trait RowIndexAndType: Sealed {
    #[doc(hidden)]
    fn _from_columns<'c>(&self, col: &'c [Column]) -> Option<(usize, &'c Type)>;
}

impl Sealed for usize {}

impl RowIndexAndType for usize {
    #[inline]
    fn _from_columns<'c>(&self, col: &'c [Column]) -> Option<(usize, &'c Type)> {
        let idx = *self;
        col.get(idx).map(|col| (idx, col.r#type()))
    }
}

impl Sealed for str {}

impl RowIndexAndType for str {
    #[inline]
    fn _from_columns<'c>(&self, col: &'c [Column]) -> Option<(usize, &'c Type)> {
        col.iter()
            .enumerate()
            .find_map(|(idx, col)| col.name().eq(self).then(|| (idx, col.r#type())))
            .or_else(|| {
                // FIXME ASCII-only case insensitivity isn't really the right thing to
                // do. Postgres itself uses a dubious wrapper around tolower and JDBC
                // uses the US locale.
                col.iter()
                    .enumerate()
                    .find_map(|(idx, col)| col.name().eq_ignore_ascii_case(self).then(|| (idx, col.r#type())))
            })
    }
}

impl<'a, T> Sealed for &'a T where T: ?Sized + Sealed {}

impl<'a, T> RowIndexAndType for &'a T
where
    T: RowIndexAndType + ?Sized,
{
    #[inline]
    fn _from_columns<'c>(&self, col: &'c [Column]) -> Option<(usize, &'c Type)> {
        T::_from_columns(*self, col)
    }
}
