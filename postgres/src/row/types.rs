use core::{fmt, ops::Range};

use std::sync::Arc;

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend::DataRowBody;
use postgres_types::FromSql;

use crate::{error::Error, statement::Column, Type};

use super::traits::RowIndex;

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
}

impl<'a> GenericRow<&'a [Column]> {
    /// Deserializes a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    #[inline]
    pub fn get<'f, T>(&'a self, idx: impl RowIndex + fmt::Display) -> T
    where
        T: FromSql<'f>,
        'a: 'f,
    {
        self.try_get(&idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// Like `Row::get`, but returns a `Result` rather than panicking.
    pub fn try_get<'f, T>(&'a self, idx: impl RowIndex + fmt::Display) -> Result<T, Error>
    where
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

impl GenericRow<Arc<[Column]>> {
    /// Returns a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    pub fn get(&self, idx: impl RowIndex + fmt::Display) -> Option<&str> {
        self.try_get(&idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// Like `SimpleQueryRow::get`, but returns a `Result` rather than panicking.
    pub fn try_get(&self, idx: impl RowIndex + fmt::Display) -> Result<Option<&str>, Error> {
        let idx = match idx.__idx(&self.columns) {
            Some(idx) => idx,
            None => return Err(Error::ToDo),
        };
        let buf = self.ranges[idx].clone().map(|r| &self.body.buffer()[r]);
        FromSql::from_sql_nullable(&Type::TEXT, buf).map_err(|_| Error::ToDo)
    }
}
