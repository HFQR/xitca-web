use core::ops::Range;

use std::error::Error;

use xitca_io::bytes::Bytes;
use xitca_unsafe_collection::bytes::BytesStr;

use super::{FromSql, Type};

pub type FromSqlError = Box<dyn Error + Sync + Send>;

pub trait FromSqlExt<'a>: Sized {
    fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Self, FromSqlError>;

    fn accepts(ty: &Type) -> bool;
}

impl<'a, T> FromSqlExt<'a> for Option<T>
where
    T: FromSql<'a>,
{
    #[inline]
    fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Self, FromSqlError> {
        <Self as FromSql>::from_sql_nullable(ty, buf.map(|(r, buf)| &buf[r.start..r.end]))
    }

    #[inline]
    fn accepts(ty: &Type) -> bool {
        <Self as FromSql>::accepts(ty)
    }
}

impl<'a, T: FromSql<'a>> FromSqlExt<'a> for Vec<T> {
    #[inline]
    fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Vec<T>, FromSqlError> {
        <Self as FromSql>::from_sql_nullable(ty, buf.map(|(r, buf)| &buf[r.start..r.end]))
    }

    #[inline]
    fn accepts(ty: &Type) -> bool {
        <Self as FromSql>::accepts(ty)
    }
}

impl<'a, T> FromSqlExt<'a> for Box<[T]>
where
    T: FromSql<'a>,
{
    #[inline]
    fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Self, FromSqlError> {
        <Self as FromSql>::from_sql_nullable(ty, buf.map(|(r, buf)| &buf[r.start..r.end]))
    }

    #[inline]
    fn accepts(ty: &Type) -> bool {
        <Self as FromSql>::accepts(ty)
    }
}

macro_rules! forward_impl {
    ($ty: ty) => {
        impl<'a> FromSqlExt<'a> for $ty {
            #[inline]
            fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Self, FromSqlError> {
                <Self as FromSql>::from_sql_nullable(ty, buf.map(|(r, buf)| &buf[r.start..r.end]))
            }

            #[inline]
            fn accepts(ty: &Type) -> bool {
                <Self as FromSql>::accepts(ty)
            }
        }
    };
}

// forward_impl!(Vec<u8>);
forward_impl!(&'a [u8]);
forward_impl!(&'a str);
forward_impl!(String);
forward_impl!(Box<str>);
forward_impl!(bool);
forward_impl!(i8);
forward_impl!(i16);
forward_impl!(i32);
forward_impl!(u32);
forward_impl!(i64);
forward_impl!(f32);
forward_impl!(f64);

impl<'a> FromSqlExt<'a> for BytesStr {
    fn from_sql_nullable_ext(ty: &Type, buf: Option<(&Range<usize>, &'a Bytes)>) -> Result<Self, FromSqlError> {
        match buf {
            Some((r, buf)) => {
                <&str as FromSql>::from_sql(ty, &buf[r.start..r.end])?;
                Ok(BytesStr::try_from(buf.slice(r.start..r.end)).unwrap())
            }
            None => <&str as FromSql>::from_sql_null(ty)
                .map(|_| unreachable!("<&str as FromSql>::from_sql_null should always yield Result::Err branch")),
        }
    }

    fn accepts(ty: &Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }
}
