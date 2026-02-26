//! re-export of postgres-protocol crate with additional query functions

use xitca_io::bytes::{BufMut, BytesMut};

use std::{convert::Infallible, io};

use super::error::Error;

pub(crate) use postgres_protocol::*;

// an optimized version of postgres_protocol::frontend::bind function
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
    buf.put_u8(b'B');

    write_body(buf, |buf| {
        write_cstr(portal_name, buf);
        write_cstr(stmt_name, buf);
        write_counted(
            formats,
            |f, buf| {
                buf.put_i16(f);
                Ok::<_, Infallible>(())
            },
            buf,
        )?;
        write_counted(values, |v, buf| write_nullable(|buf| serializer(v, buf), buf), buf)?;
        write_counted(
            result_formats.into_iter(),
            |f, buf| {
                buf.put_i16(f);
                Ok::<_, Infallible>(())
            },
            buf,
        )
    })
}

fn write_body<F>(buf: &mut BytesMut, f: F) -> Result<(), Error>
where
    F: FnOnce(&mut BytesMut) -> Result<(), Error>,
{
    let base = buf.len();
    buf.put_i32(0);

    f(buf)?;

    let size = FromUsize::from_usize(buf.len() - base)?;
    buf.put_i32_at(base, size);
    Ok(())
}

fn write_counted<I, T, F, E>(items: I, mut serializer: F, buf: &mut BytesMut) -> Result<(), Error>
where
    I: ExactSizeIterator<Item = T>,
    F: FnMut(T, &mut BytesMut) -> Result<(), E>,
    Error: From<E>,
{
    let count = FromUsize::from_usize(items.len())?;
    buf.put_u16(count);

    for item in items {
        serializer(item, buf)?;
    }

    Ok(())
}

fn write_nullable<F>(serializer: F, buf: &mut BytesMut) -> Result<(), Error>
where
    F: FnOnce(&mut BytesMut) -> Result<IsNull, Error>,
{
    let base = buf.len();
    buf.put_i32(0);

    let size = match serializer(buf)? {
        IsNull::No => FromUsize::from_usize(buf.len() - base - 4)?,
        IsNull::Yes => -1,
    };
    buf.put_i32_at(base, size);

    Ok(())
}

fn write_cstr(s: &str, buf: &mut BytesMut) {
    // strings used inside library dont contain c style null bytes.
    // debug assertions is enough for catching violation.
    #[cfg(debug_assertions)]
    if s.as_bytes().contains(&0) {
        panic!("input string: {s} contains embedded null")
    }

    buf.put_slice(s.as_bytes());
    buf.put_u8(0);
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
