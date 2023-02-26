use core::ops::Range;

use xitca_io::bytes::Bytes;
use xitca_unsafe_collection::bytes::BytesStr;

use super::{FromSql, Type};

pub type FromSqlError = Box<dyn std::error::Error + Sync + Send>;

/// extension trait for [FromSql].
/// instead of working with explicit reference as `&[u8]` for parsing raw sql bytes this extension
/// offers cheap copy/slicing of [Bytes] type for reference counting based zero copy parsing.
///
/// # Examples
/// ```rust
/// # use xitca_postgres::Row;
/// use xitca_unsafe_collection::bytes::BytesStr; // a reference counted &str type.
/// fn parse_row(row: Row<'_>) {
///     let s = row.get::<BytesStr>(0); // parse index0 column with zero copy.
///     println!("{}", s.as_str());
/// }
/// ```
pub trait FromSqlExt<'a>: Sized {
    /// [Type] represents the Postgres type hint which Self must be matching.
    /// [Bytes] represents the reference of raw bytes of row data Self belongs to.
    /// [Range] represents the start and end indexing into the raw data for correctly parsing Self.
    /// Option::None variant of range and bytes hints current value can be a null type.
    fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Self, FromSqlError>;

    /// Determines if a value of this type can be created from the specified Postgres [Type].
    fn accepts(ty: &Type) -> bool;
}

macro_rules! default_impl {
    ($ty: ty) => {
        impl<'a> FromSqlExt<'a> for $ty {
            default_impl!();
        }
    };
    () => {
        #[inline]
        fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Self, FromSqlError> {
            <Self as FromSql>::from_sql_nullable(ty, buf.map(|(r, buf)| &buf[r.start..r.end]))
        }

        #[inline]
        fn accepts(ty: &Type) -> bool {
            <Self as FromSql>::accepts(ty)
        }
    };
}

impl<'a, T> FromSqlExt<'a> for Option<T>
where
    T: FromSql<'a>,
{
    default_impl!();
}

impl<'a, T> FromSqlExt<'a> for Vec<T>
where
    T: FromSql<'a>,
{
    default_impl!();
}

impl<'a, T> FromSqlExt<'a> for Box<[T]>
where
    T: FromSql<'a>,
{
    default_impl!();
}

// forward_impl!(Vec<u8>);
default_impl!(&'a [u8]);
default_impl!(&'a str);
default_impl!(String);
default_impl!(Box<str>);
default_impl!(bool);
default_impl!(i8);
default_impl!(i16);
default_impl!(i32);
default_impl!(u32);
default_impl!(i64);
default_impl!(f32);
default_impl!(f64);

impl<'a> FromSqlExt<'a> for BytesStr {
    fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Self, FromSqlError> {
        // copy/paste from postgres-protocol dependency.
        fn adjust_range(name: &str, range: &Range<usize>, buf: &Bytes) -> Result<(usize, usize), FromSqlError> {
            if buf[range.start] == 1u8 {
                Ok((range.start + 1, range.end))
            } else {
                Err(format!("only {name} version 1 supported").into())
            }
        }

        match buf {
            Some((range, buf)) => {
                let (start, end) = match ty.name() {
                    "ltree" => adjust_range("ltree", range, buf)?,
                    "lquery" => adjust_range("lquery", range, buf)?,
                    "ltxtquery" => adjust_range("ltxtquery", range, buf)?,
                    _ => (range.start, range.end),
                };
                BytesStr::try_from(buf.slice(start..end)).map_err(Into::into)
            }
            None => <&str as FromSql>::from_sql_null(ty)
                .map(|_| unreachable!("<&str as FromSql>::from_sql_null should always yield Result::Err branch")),
        }
    }

    #[inline]
    fn accepts(ty: &Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }
}

impl<'a> FromSqlExt<'a> for Bytes {
    fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Self, FromSqlError> {
        match buf {
            Some((r, buf)) => Ok(buf.slice(r.start..r.end)),
            None => <&[u8] as FromSql>::from_sql_null(ty)
                .map(|_| unreachable!("<&[u8] as FromSql>::from_sql_null should always yield Result::Err branch")),
        }
    }

    #[inline]
    fn accepts(ty: &Type) -> bool {
        <&[u8] as FromSql>::accepts(ty)
    }
}
