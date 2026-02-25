//! re-export of postgres-protocol crate with additional query functions

use xitca_io::bytes::{BufMut, BytesMut};

use std::io;

use super::error::Error;

pub(crate) use postgres_protocol::*;

// an optimized version of postgres_protocol::frontend::bind function
#[inline]
pub(crate) fn bind<I, J, F, T, K>(
    portal_name: &str,
    stmt_name: &str,
    formats: I,
    values: J,
    mut serializer: F,
    result_formats: K,
    buf: &mut BytesMut,
) -> Result<(), Error>
where
    I: ExactSizeIterator<Item = i16>,
    J: ExactSizeIterator<Item = T>,
    F: FnMut(T, &mut BytesMut) -> Result<IsNull, Error>,
    K: IntoIterator,
    K::IntoIter: ExactSizeIterator<Item = i16>,
{
    // portal and statement names are constructed inside library boundary and they dont contain c style null bytes.
    // debug assertions is enough for catching violation.
    #[cfg(debug_assertions)]
    validate_cstr([portal_name, stmt_name]);

    buf.put_u8(b'B');

    write_body(buf, |buf| {
        write_cstr(portal_name.as_bytes(), buf);
        write_cstr(stmt_name.as_bytes(), buf);
        write_counted(
            formats,
            |f, buf| {
                buf.put_i16(f);
                Ok(())
            },
            buf,
        )?;
        write_counted(values, |v, buf| write_nullable(|buf| serializer(v, buf), buf), buf)?;
        write_counted(
            result_formats.into_iter(),
            |f, buf| {
                buf.put_i16(f);
                Ok(())
            },
            buf,
        )?;

        Ok(())
    })
}

#[inline]
fn write_body<F, E>(buf: &mut BytesMut, f: F) -> Result<(), E>
where
    F: FnOnce(&mut BytesMut) -> Result<(), E>,
    E: From<io::Error>,
{
    let base = buf.len();
    buf.put_i32(0);

    f(buf)?;

    let size = i32::from_usize(buf.len() - base)?;
    buf.put_i32_at(base, size);
    Ok(())
}

#[inline]
fn write_counted<I, T, F>(items: I, mut serializer: F, buf: &mut BytesMut) -> Result<(), Error>
where
    I: ExactSizeIterator<Item = T>,
    F: FnMut(T, &mut BytesMut) -> Result<(), Error>,
{
    let count = u16::from_usize(items.len())?;
    buf.put_u16(count);

    for item in items {
        serializer(item, buf)?;
    }

    Ok(())
}

fn write_nullable<F, E>(serializer: F, buf: &mut BytesMut) -> Result<(), E>
where
    F: FnOnce(&mut BytesMut) -> Result<IsNull, E>,
    E: From<io::Error>,
{
    let base = buf.len();
    buf.put_i32(0);

    let size = match serializer(buf)? {
        IsNull::No => i32::from_usize(buf.len() - base - 4)?,
        IsNull::Yes => -1,
    };
    buf.put_i32_at(base, size);

    Ok(())
}

#[inline]
fn write_cstr(s: &[u8], buf: &mut BytesMut) {
    buf.put_slice(s);
    buf.put_u8(0);
}

#[cfg(debug_assertions)]
fn validate_cstr<'s, S>(s: S)
where
    S: IntoIterator,
    S::IntoIter: Iterator<Item = &'s str>,
{
    for s in s {
        if s.as_bytes().contains(&0) {
            panic!("input string: {s} contains embedded null")
        }
    }
}

trait FromUsize: Sized {
    fn from_usize(x: usize) -> Result<Self, io::Error>;
}

macro_rules! from_usize {
    ($t:ty) => {
        impl FromUsize for $t {
            #[inline]
            fn from_usize(x: usize) -> io::Result<$t> {
                if x > <$t>::MAX as usize {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "value too large to transmit",
                    ))
                } else {
                    Ok(x as $t)
                }
            }
        }
    };
}

from_usize!(i16);
from_usize!(u16);
from_usize!(i32);

trait WriteSize {
    fn put_i32_at(&mut self, offset: usize, num: i32);
}

impl WriteSize for BytesMut {
    fn put_i32_at(&mut self, offset: usize, num: i32) {
        self[offset..offset + 4].copy_from_slice(&num.to_be_bytes());
    }
}
