//! Rows. mostly copy/paste from `tokio-postgres`

use std::{fmt, ops::Range, str};

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend;
use postgres_types::FromSql;

use super::{
    error::Error,
    statement::{Column, Statement},
};

use self::sealed::{AsName, Sealed};

mod sealed {
    pub trait Sealed {}

    pub trait AsName {
        fn as_name(&self) -> &str;
    }
}

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

/// A row of data returned from the database by a query.
pub struct Row<'a> {
    stmt: &'a Statement,
    body: backend::DataRowBody,
    ranges: Vec<Option<Range<usize>>>,
}

impl fmt::Debug for Row<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Row").field("columns", &self.columns()).finish()
    }
}

impl<'a> Row<'a> {
    pub(crate) fn try_new(stmt: &'a Statement, body: backend::DataRowBody) -> Result<Self, Error> {
        let ranges = body.ranges().collect().map_err(|_| Error::ToDo)?;
        Ok(Row { stmt, body, ranges })
    }

    /// Returns information about the columns of data in the row.
    pub fn columns(&self) -> &[Column] {
        self.stmt.columns()
    }

    /// Determines if the row contains no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of values in the row.
    pub fn len(&self) -> usize {
        self.columns().len()
    }

    /// Deserializes a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    #[inline]
    pub fn get<'f, I, T>(&'a self, idx: I) -> T
    where
        I: RowIndex + fmt::Display,
        T: FromSql<'f>,
        'a: 'f,
    {
        self.try_get(&idx)
            .unwrap_or_else(|e| panic!("error retrieving column {}: {}", idx, e))
    }

    /// Like `Row::get`, but returns a `Result` rather than panicking.
    pub fn try_get<'f, I, T>(&'a self, idx: I) -> Result<T, Error>
    where
        I: RowIndex + fmt::Display,
        T: FromSql<'f>,
        'a: 'f,
    {
        let idx = match idx.__idx(self.columns()) {
            Some(idx) => idx,
            // None => return Err(Error::column(idx.to_string())),
            None => return Err(Error::ToDo),
        };

        let ty = self.columns()[idx].type_();
        if !T::accepts(ty) {
            return Err(Error::ToDo);
            // return Err(Error::from_sql(Box::new(WrongType::new::<T>(ty.clone())), idx));
        }

        FromSql::from_sql_nullable(ty, self.col_buffer(idx)).map_err(|_| Error::ToDo)
    }

    /// Get the raw bytes for the column at the given index.
    fn col_buffer(&self, idx: usize) -> Option<&[u8]> {
        let range = self.ranges[idx].to_owned()?;
        Some(&self.body.buffer()[range])
    }
}
