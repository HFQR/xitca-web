use core::ops::Range;

use xitca_io::bytes::Bytes;
use xitca_unsafe_collection::bytes::BytesStr;

use super::types::{FromSql, Type, WasNull};

pub type FromSqlError = Box<dyn core::error::Error + Sync + Send>;

/// extension trait for [FromSql]
///
/// instead of working with explicit reference as `&[u8]` for parsing raw sql bytes this extension
/// offers cheap copy/slicing of [Bytes] type for reference counting based zero copy parsing.
///
/// # Examples
/// ```rust
/// # use xitca_postgres::row::Row;
/// use xitca_unsafe_collection::bytes::BytesStr; // a reference counted &str type has FromSqlExt impl
/// fn parse_row(row: Row<'_>) {
///     let s = row.get_zc::<BytesStr>(0); // parse index0 column with zero copy.
///     println!("{}", s.as_str());
/// }
/// ```
pub trait FromSqlExt<'a>: Sized {
    /// byte parser of non null Postgres type
    /// [Type] represents the Postgres type hint which Self must be matching.
    /// [Bytes] represents the reference of raw bytes of row data Self belongs to.
    /// [Range] represents the start and end indexing into the raw data for correctly parsing Self.
    fn from_sql_ext(ty: &Type, col: (&Range<usize>, &'a Bytes)) -> Result<Self, FromSqlError>;

    /// Creates a new value of this type from a `NULL` SQL value.
    ///
    /// The caller of this method is responsible for ensuring that this type
    /// is compatible with the Postgres `Type`.
    ///
    /// The default implementation returns `Err(Box::new(WasNull))`.
    #[allow(unused_variables)]
    #[inline]
    fn from_sql_null_ext(ty: &Type) -> Result<Self, FromSqlError> {
        Err(Box::new(WasNull))
    }

    /// A convenience function that delegates to `from_sql` and `from_sql_null`.
    fn from_sql_nullable_ext(ty: &Type, col: (&Range<usize>, &'a Bytes)) -> Result<Self, FromSqlError> {
        match col.0.is_empty() {
            false => Self::from_sql_ext(ty, col),
            true => Self::from_sql_null_ext(ty),
        }
    }

    /// Determines if a value of this type can be created from the specified Postgres `Type`.
    fn accepts_ext(ty: &Type) -> bool;
}

impl<'a> FromSqlExt<'a> for BytesStr {
    fn from_sql_ext(ty: &Type, (range, buf): (&Range<usize>, &'a Bytes)) -> Result<Self, FromSqlError> {
        // copy/paste from postgres-protocol dependency.
        fn adjust_range(name: &str, range: &Range<usize>, buf: &Bytes) -> Result<(usize, usize), FromSqlError> {
            if buf[range.start] == 1u8 {
                Ok((range.start + 1, range.end))
            } else {
                Err(format!("only {name} version 1 supported").into())
            }
        }

        let (start, end) = match ty.name() {
            "ltree" => adjust_range("ltree", range, buf)?,
            "lquery" => adjust_range("lquery", range, buf)?,
            "ltxtquery" => adjust_range("ltxtquery", range, buf)?,
            _ => (range.start, range.end),
        };

        BytesStr::try_from(buf.slice(start..end)).map_err(Into::into)
    }

    #[inline]
    fn accepts_ext(ty: &Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }
}

impl<'a> FromSqlExt<'a> for Bytes {
    #[inline]
    fn from_sql_ext(_: &Type, (range, buf): (&Range<usize>, &'a Bytes)) -> Result<Self, FromSqlError> {
        Ok(buf.slice(range.start..range.end))
    }

    #[inline]
    fn accepts_ext(ty: &Type) -> bool {
        <&[u8] as FromSql>::accepts(ty)
    }
}

impl<'a, T> FromSqlExt<'a> for Option<T>
where
    T: FromSqlExt<'a>,
{
    #[inline]
    fn from_sql_ext(ty: &Type, raw: (&Range<usize>, &'a Bytes)) -> Result<Option<T>, FromSqlError> {
        <T as FromSqlExt>::from_sql_ext(ty, raw).map(Some)
    }

    #[inline]
    fn from_sql_null_ext(_: &Type) -> Result<Option<T>, FromSqlError> {
        Ok(None)
    }

    #[inline]
    fn accepts_ext(ty: &Type) -> bool {
        <T as FromSqlExt>::accepts_ext(ty)
    }
}
