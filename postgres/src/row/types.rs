use core::{fmt, marker::PhantomData, ops::Range};

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend::DataRowBody;
use postgres_types::FromSql;
use xitca_io::bytes::Bytes;

use crate::{
    column::Column,
    error::{Error, InvalidColumnIndex, WrongType},
    from_sql::FromSqlExt,
    types::Type,
};

use super::traits::RowIndexAndType;

/// A row of data returned from the database by a query.
pub type Row<'r> = GenericRow<&'r [Column], &'r mut Vec<Range<usize>>, marker::Typed>;

/// A row of data returned from the database by a simple query.
pub type RowSimple<'r> = GenericRow<&'r [Column], &'r mut Vec<Range<usize>>, marker::NoTyped>;

/// Marker types for specialized impl on [GenericRow].
pub(super) mod marker {
    pub struct Typed;
    pub struct NoTyped;
}

pub struct GenericRow<C, R, M> {
    columns: C,
    body: DataRowBody,
    ranges: R,
    _marker: PhantomData<M>,
}

impl<C, R, M> fmt::Debug for GenericRow<C, R, M>
where
    C: AsRef<[Column]>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Row").field("columns", &self.columns.as_ref()).finish()
    }
}

impl<C, R, M> GenericRow<C, R, M>
where
    C: AsRef<[Column]>,
    R: AsRef<[Range<usize>]> + AsMut<Vec<Range<usize>>>,
{
    pub(crate) fn try_new(columns: C, body: DataRowBody, mut ranges: R) -> Result<Self, Error> {
        let mut iter = body.ranges();

        let ranges_mut = ranges.as_mut();

        ranges_mut.clear();
        ranges_mut.reserve(iter.size_hint().0);

        while let Some(range) = iter.next()? {
            /*
                when unwrapping the Range an empty value is used to represent null pg value offsets inside row's raw
                data buffer.
                when empty range is used to slice data collection through a safe Rust API(`<&[u8]>::get(Range<usize>)`
                in this case) it always produce Option type where the None variant can be used as final output of null
                pg value.
                this saves 8 bytes per range storage
            */
            ranges_mut.push(range.unwrap_or(Range { start: 1, end: 0 }));
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
        self.columns.as_ref()
    }

    /// Determines if the row contains no values.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of values in the row.
    #[inline]
    pub fn len(&self) -> usize {
        self.columns().len()
    }

    // Get the raw bytes for the column at the given range.
    fn col_buffer(&self, idx: usize) -> (&Range<usize>, &Bytes) {
        (&self.ranges.as_ref()[idx], self.body.buffer_bytes())
    }

    fn get_idx_ty<T>(
        &self,
        idx: impl RowIndexAndType + fmt::Display,
        ty_check: impl FnOnce(&Type) -> bool,
    ) -> Result<(usize, &Type), Error> {
        let (idx, ty) = idx
            ._from_columns(self.columns.as_ref())
            .ok_or_else(|| InvalidColumnIndex(idx.to_string()))?;

        if !ty_check(ty) {
            return Err(Error::from(WrongType::new::<T>(ty.clone())));
        }

        Ok((idx, ty))
    }
}

impl<C, R> GenericRow<C, R, marker::Typed>
where
    C: AsRef<[Column]>,
    R: AsRef<[Range<usize>]> + AsMut<Vec<Range<usize>>>,
{
    /// zero copy version of [`Self::get`]
    ///
    /// see [`FromSqlExt`] trait for explanation
    pub fn get_zc<'s, T>(&'s self, idx: impl RowIndexAndType + fmt::Display) -> T
    where
        T: FromSqlExt<'s>,
    {
        self.try_get_zc(idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// Like [`Self::get_zc`], but returns a `Result` rather than panicking.
    pub fn try_get_zc<'s, T>(&'s self, idx: impl RowIndexAndType + fmt::Display) -> Result<T, Error>
    where
        T: FromSqlExt<'s>,
    {
        let (idx, ty) = self.get_idx_ty::<T>(idx, T::accepts)?;
        FromSqlExt::from_sql_nullable_ext(ty, self.col_buffer(idx)).map_err(Into::into)
    }

    /// Deserializes a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    pub fn get<'s, T>(&'s self, idx: impl RowIndexAndType + fmt::Display) -> T
    where
        T: FromSql<'s>,
    {
        self.try_get(idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// Like [`Self::get`], but returns a `Result` rather than panicking.
    pub fn try_get<'s, T>(&'s self, idx: impl RowIndexAndType + fmt::Display) -> Result<T, Error>
    where
        T: FromSql<'s>,
    {
        let (idx, ty) = self.get_idx_ty::<T>(idx, T::accepts)?;
        FromSql::from_sql_nullable(ty, self.body.buffer().get(self.ranges.as_ref()[idx].clone())).map_err(Into::into)
    }
}

impl RowSimple<'_> {
    /// Returns a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    pub fn get(&self, idx: impl RowIndexAndType + fmt::Display) -> Option<&str> {
        self.try_get(idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// Like `RowSimple::get`, but returns a `Result` rather than panicking.
    pub fn try_get(&self, idx: impl RowIndexAndType + fmt::Display) -> Result<Option<&str>, Error> {
        let (idx, ty) = self.get_idx_ty::<&str>(idx, <&str as FromSql>::accepts)?;
        FromSqlExt::from_sql_nullable_ext(ty, self.col_buffer(idx)).map_err(Into::into)
    }
}

fn _try_get_usize(row: Row) {
    let _ = row.try_get_zc::<u32>(0);
    let _ = row.try_get_zc::<&str>("test");
    let _ = row.try_get::<String>(String::from("get_raw").as_str());
}
