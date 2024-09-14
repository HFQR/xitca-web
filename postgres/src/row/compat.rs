//! compatible types for Row that can work with futures::Stream trait

use core::{fmt, ops::Range};

use std::sync::Arc;

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend::DataRowBody;

use crate::{
    column::Column,
    error::{Error, InvalidColumnIndex, WrongType},
    types::{FromSql, Type},
};

use super::traits::RowIndexAndType;

pub struct RowOwned {
    columns: Arc<[Column]>,
    body: DataRowBody,
    ranges: Vec<Range<usize>>,
}

impl RowOwned {
    pub(crate) fn try_new(columns: Arc<[Column]>, body: DataRowBody) -> Result<Self, Error> {
        let mut iter = body.ranges();

        let mut ranges = Vec::with_capacity(iter.size_hint().0);

        while let Some(range) = iter.next()? {
            /*
                when unwrapping the Range an empty value is used to represent null pg value offsets inside row's raw
                data buffer.
                when empty range is used to slice data collection through a safe Rust API(`<&[u8]>::get(Range<usize>)`
                in this case) it always produce Option type where the None variant can be used as final output of null
                pg value.
                this saves 8 bytes per range storage
            */
            ranges.push(range.unwrap_or(Range { start: 1, end: 0 }));
        }

        Ok(Self { columns, body, ranges })
    }

    /// Returns information about the columns of data in the row.
    #[inline]
    pub fn columns(&self) -> &[Column] {
        &self.columns
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

    /// hidden api for get row data with [FromSql] trait implementation.
    pub fn get<'s, T>(&'s self, idx: impl RowIndexAndType + fmt::Display) -> T
    where
        T: FromSql<'s>,
    {
        self.try_get(idx)
            .unwrap_or_else(|e| panic!("error retrieving column {idx}: {e}"))
    }

    /// hidden api for get row data with [FromSql] trait implementation.
    pub fn try_get<'s, T>(&'s self, idx: impl RowIndexAndType + fmt::Display) -> Result<T, Error>
    where
        T: FromSql<'s>,
    {
        let (idx, ty) = self.get_idx_ty::<T>(idx, T::accepts)?;
        FromSql::from_sql_nullable(ty, self.body.buffer().get(self.ranges[idx].clone())).map_err(Into::into)
    }

    fn get_idx_ty<T>(
        &self,
        idx: impl RowIndexAndType + fmt::Display,
        ty_check: impl FnOnce(&Type) -> bool,
    ) -> Result<(usize, &Type), Error> {
        let (idx, ty) = idx
            ._from_columns(&self.columns)
            .ok_or_else(|| InvalidColumnIndex(idx.to_string()))?;

        if !ty_check(ty) {
            return Err(Error::from(WrongType::new::<T>(ty.clone())));
        }

        Ok((idx, ty))
    }
}
