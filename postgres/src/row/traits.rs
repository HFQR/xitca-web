use crate::column::Column;

mod sealed {
    pub trait Sealed {}

    pub trait AsName {
        fn as_name(&self) -> &str;
    }
}

use self::sealed::{AsName, Sealed};

impl AsName for Column {
    fn as_name(&self) -> &str {
        self.name()
    }
}

impl AsName for String {
    fn as_name(&self) -> &str {
        self
    }
}

/// A trait implemented by types that can index into columns of a row.
///
/// This cannot be implemented outside of this crate.
pub trait RowIndex: Sealed {
    #[doc(hidden)]
    fn __idx<T>(&self, columns: &[T]) -> Option<usize>
    where
        T: AsName;
}

impl Sealed for usize {}

impl RowIndex for usize {
    #[inline]
    fn __idx<T>(&self, columns: &[T]) -> Option<usize>
    where
        T: AsName,
    {
        if *self >= columns.len() {
            None
        } else {
            Some(*self)
        }
    }
}

impl Sealed for str {}

impl RowIndex for str {
    #[inline]
    fn __idx<T>(&self, columns: &[T]) -> Option<usize>
    where
        T: AsName,
    {
        if let Some(idx) = columns.iter().position(|d| d.as_name() == self) {
            return Some(idx);
        };

        // FIXME ASCII-only case insensitivity isn't really the right thing to
        // do. Postgres itself uses a dubious wrapper around tolower and JDBC
        // uses the US locale.
        columns.iter().position(|d| d.as_name().eq_ignore_ascii_case(self))
    }
}

impl<'a, T> Sealed for &'a T where T: ?Sized + Sealed {}

impl<'a, T> RowIndex for &'a T
where
    T: ?Sized + RowIndex,
{
    #[inline]
    fn __idx<U>(&self, columns: &[U]) -> Option<usize>
    where
        U: AsName,
    {
        T::__idx(*self, columns)
    }
}
