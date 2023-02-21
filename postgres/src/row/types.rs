use core::{fmt, marker::PhantomData, ops::Range};

use std::sync::Arc;

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend::DataRowBody;
use postgres_types::FromSql;

use crate::{column::Column, error::Error, Type};

use super::traits::RowIndexAndType;

/// A row of data returned from the database by a query.
pub type Row<'a> = GenericRow<&'a [Column]>;

/// A row of data returned from the database by a simple query.
pub type RowSimple = GenericRow<Arc<[Column]>>;

pub struct GenericRow<C> {
    columns: C,
    body: DataRowBody,
    ranges: Vec<Option<Range<usize>>>,
}

impl<C> fmt::Debug for GenericRow<C>
where
    C: AsRef<[Column]>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Row").field("columns", &self.columns()).finish()
    }
}

impl<C> GenericRow<C>
where
    C: AsRef<[Column]>,
{
    pub(crate) fn try_new(columns: C, body: DataRowBody) -> Result<Self, Error> {
        let ranges = body.ranges().collect().map_err(|_| Error::ToDo)?;
        Ok(Self { columns, body, ranges })
    }

    /// Returns information about the columns of data in the row.
    #[inline]
    pub fn columns(&self) -> &[Column] {
        self.columns.as_ref()
    }

    /// Determines if the row contains no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of values in the row.
    pub fn len(&self) -> usize {
        self.columns().len()
    }

    // Get the raw bytes for the column at the given index.
    fn col_buffer(&self, idx: usize) -> Option<&[u8]> {
        self.ranges[idx].to_owned().map(|range| &self.body.buffer()[range])
    }
}

impl<'a> Row<'a> {
    /// Deserializes a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    #[inline]
    pub fn get<'f, T>(&'a self, idx: impl RowIndexAndType + fmt::Display) -> T
    where
        T: FromSql<'f>,
        'a: 'f,
    {
        self.try_get(&idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// Like `Row::get`, but returns a `Result` rather than panicking.
    pub fn try_get<'f, T>(&'a self, idx: impl RowIndexAndType + fmt::Display) -> Result<T, Error>
    where
        T: FromSql<'f>,
        'a: 'f,
    {
        let (idx, ty) = idx.__from_columns(self.columns()).ok_or(Error::ToDo)?;

        if !T::accepts(ty) {
            return Err(Error::ToDo);
            // return Err(Error::from_sql(Box::new(WrongType::new::<T>(ty.clone())), idx));
        }

        FromSql::from_sql_nullable(ty, self.col_buffer(idx)).map_err(|_| Error::ToDo)
    }
}

impl RowSimple {
    /// Returns a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    pub fn get(&self, idx: impl RowIndexAndType + fmt::Display) -> Option<&str> {
        self.try_get(&idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// Like `SimpleQueryRow::get`, but returns a `Result` rather than panicking.
    pub fn try_get(&self, idx: impl RowIndexAndType + fmt::Display) -> Result<Option<&str>, Error> {
        let (idx, _) = idx.__from_columns(self.columns()).ok_or(Error::ToDo)?;
        FromSql::from_sql_nullable(&Type::TEXT, self.col_buffer(idx)).map_err(|_| Error::ToDo)
    }
}

/// A row of data returned from the database by a query.
pub type RowGat<'r> = GenericRowGat<'r, marker::Typed>;

/// A row of data returned from the database by a simple query.
pub type RowSimpleGat<'r> = GenericRowGat<'r, marker::NoTyped>;

/// Marker types for specialized impl on [GenericRowGat].
mod marker {
    pub struct Typed;
    pub struct NoTyped;
}

pub struct GenericRowGat<'a, M> {
    columns: &'a [Column],
    body: DataRowBody,
    ranges: &'a mut Vec<Option<Range<usize>>>,
    _marker: PhantomData<M>,
}

impl<M> fmt::Debug for GenericRowGat<'_, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Row").field("columns", &self.columns()).finish()
    }
}

impl<'a, C> GenericRowGat<'a, C> {
    pub(crate) fn try_new(
        columns: &'a [Column],
        body: DataRowBody,
        ranges: &'a mut Vec<Option<Range<usize>>>,
    ) -> Result<Self, Error> {
        ranges.clear();
        let mut iter = body.ranges();
        ranges.reserve(iter.size_hint().0);
        while let Some(range) = iter.next()? {
            ranges.push(range);
        }
        Ok(Self {
            columns,
            body,
            ranges,
            _marker: PhantomData,
        })
    }

    /// Returns information about the columns of data in the row.
    #[inline]
    pub fn columns(&self) -> &[Column] {
        self.columns
    }

    /// Determines if the row contains no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of values in the row.
    pub fn len(&self) -> usize {
        self.columns().len()
    }

    // Get the raw bytes for the column at the given index.
    fn col_buffer(&self, idx: usize) -> Option<&[u8]> {
        self.ranges[idx].to_owned().map(|range| &self.body.buffer()[range])
    }
}

impl<'r> RowGat<'r> {
    /// Deserializes a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    #[inline]
    pub fn get<'f, T>(&'r self, idx: impl RowIndexAndType + fmt::Display) -> T
    where
        T: FromSql<'f>,
        'r: 'f,
    {
        self.try_get(&idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// Like `Row::get`, but returns a `Result` rather than panicking.
    pub fn try_get<'f, T>(&'r self, idx: impl RowIndexAndType + fmt::Display) -> Result<T, Error>
    where
        T: FromSql<'f>,
        'r: 'f,
    {
        let (idx, ty) = idx.__from_columns(self.columns()).ok_or(Error::ToDo)?;

        if !T::accepts(ty) {
            return Err(Error::ToDo);
            // return Err(Error::from_sql(Box::new(WrongType::new::<T>(ty.clone())), idx));
        }

        FromSql::from_sql_nullable(ty, self.col_buffer(idx)).map_err(|_| Error::ToDo)
    }
}

impl RowSimpleGat<'_> {
    /// Returns a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    pub fn get(&self, idx: impl RowIndexAndType + fmt::Display) -> Option<&str> {
        self.try_get(&idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// Like `SimpleQueryRow::get`, but returns a `Result` rather than panicking.
    pub fn try_get(&self, idx: impl RowIndexAndType + fmt::Display) -> Result<Option<&str>, Error> {
        let (idx, _) = idx.__from_columns(self.columns()).ok_or(Error::ToDo)?;
        FromSql::from_sql_nullable(&Type::TEXT, self.col_buffer(idx)).map_err(|_| Error::ToDo)
    }
}
