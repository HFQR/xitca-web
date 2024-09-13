use crate::{column::Column, types::Type};

mod sealed {
    pub trait Sealed {}
}

#[doc(hidden)]
/// a trait implemented by types that can find index and it's associated [Type] into columns of a
/// row. cannot be implemented beyond crate boundary.
pub trait RowIndexAndType: sealed::Sealed + Copy {
    fn _from_columns(self, col: &[Column]) -> Option<(usize, &Type)>;
}

impl sealed::Sealed for usize {}

impl RowIndexAndType for usize {
    #[inline]
    fn _from_columns(self, col: &[Column]) -> Option<(usize, &Type)> {
        col.get(self).map(|col| (self, col.r#type()))
    }
}

impl sealed::Sealed for &str {}

impl RowIndexAndType for &str {
    #[inline]
    fn _from_columns(self, col: &[Column]) -> Option<(usize, &Type)> {
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
