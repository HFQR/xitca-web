use core::ops::Range;

use xitca_io::bytes::Bytes;
use xitca_unsafe_collection::bytes::BytesStr;

use super::types::{FromSql, Type};

pub type FromSqlError = Box<dyn std::error::Error + Sync + Send>;

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
    /// [Type] represents the Postgres type hint which Self must be matching.
    /// [Bytes] represents the reference of raw bytes of row data Self belongs to.
    /// [Range] represents the start and end indexing into the raw data for correctly parsing Self.
    /// When [Range] is an empty value it indicates trait implementor encounters a null pg value. It's
    /// suggested to call [Range::is_empty] to check for this case and properly handle it
    fn from_sql_nullable_ext(ty: &Type, col: (&Range<usize>, &'a Bytes)) -> Result<Self, FromSqlError>;

    /// Determines if a value of this type can be created from the specified Postgres [Type].
    fn accepts(ty: &Type) -> bool;
}

impl<'a> FromSqlExt<'a> for BytesStr {
    fn from_sql_nullable_ext(ty: &Type, (range, buf): (&Range<usize>, &'a Bytes)) -> Result<Self, FromSqlError> {
        // copy/paste from postgres-protocol dependency.
        fn adjust_range(name: &str, range: &Range<usize>, buf: &Bytes) -> Result<(usize, usize), FromSqlError> {
            if buf[range.start] == 1u8 {
                Ok((range.start + 1, range.end))
            } else {
                Err(format!("only {name} version 1 supported").into())
            }
        }

        if range.is_empty() {
            return <&str as FromSql>::from_sql_null(ty)
                .map(|_| unreachable!("<&str as FromSql>::from_sql_null should always yield Result::Err branch"));
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
    fn accepts(ty: &Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }
}

impl<'a> FromSqlExt<'a> for Option<BytesStr> {
    #[inline]
    fn from_sql_nullable_ext(ty: &Type, col: (&Range<usize>, &'a Bytes)) -> Result<Self, FromSqlError> {
        BytesStr::from_sql_nullable_ext(ty, col).map(Some)
    }

    #[inline]
    fn accepts(ty: &Type) -> bool {
        <BytesStr as FromSqlExt>::accepts(ty)
    }
}

impl<'a> FromSqlExt<'a> for Bytes {
    #[inline]
    fn from_sql_nullable_ext(ty: &Type, (range, buf): (&Range<usize>, &'a Bytes)) -> Result<Self, FromSqlError> {
        if range.is_empty() {
            return <&[u8] as FromSql>::from_sql_null(ty)
                .map(|_| unreachable!("<&[u8] as FromSql>::from_sql_null should always yield Result::Err branch"));
        }

        Ok(buf.slice(range.start..range.end))
    }

    #[inline]
    fn accepts(ty: &Type) -> bool {
        <&[u8] as FromSql>::accepts(ty)
    }
}

impl<'a> FromSqlExt<'a> for Option<Bytes> {
    #[inline]
    fn from_sql_nullable_ext(ty: &Type, col: (&Range<usize>, &'a Bytes)) -> Result<Self, FromSqlError> {
        Bytes::from_sql_nullable_ext(ty, col).map(Some)
    }

    #[inline]
    fn accepts(ty: &Type) -> bool {
        <Bytes as FromSqlExt>::accepts(ty)
    }
}
