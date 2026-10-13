use core::fmt;

use crate::bytes::{Buf, BufMut};

use crate::h3::proto::coding::{self, BufExt, BufMutExt};

#[derive(Debug, PartialEq)]
pub enum Error {
    Overflow,
    UnexpectedEnd,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Overflow => write!(f, "value overflow"),
            Error::UnexpectedEnd => write!(f, "unexpected end"),
        }
    }
}

pub fn decode<B: Buf>(size: u8, buf: &mut B) -> Result<(u8, u64), Error> {
    assert!(size <= 8);
    let mut first = buf.get::<u8>()?;

    // NOTE: following casts to u8 intend to trim the most significant bits, they are used as a
    //       workaround for shiftoverflow errors when size == 8.
    let flags = ((first as usize) >> size) as u8;
    let mask = 0xFF >> (8 - size);
    first &= mask;

    // if first < 2usize.pow(size) - 1
    if first < mask {
        return Ok((flags, first as u64));
    }

    let mut value = mask as u64;
    let mut power = 0usize;
    loop {
        let byte = buf.get::<u8>()? as u64;
        value += (byte & 127) << power;
        power += 7;

        if byte & 128 == 0 {
            break;
        }

        if power >= MAX_POWER {
            return Err(Error::Overflow);
        }
    }

    Ok((flags, value))
}

pub fn encode<B: BufMut>(size: u8, flags: u8, value: u64, buf: &mut B) {
    assert!(size <= 8);
    // NOTE: following casts to u8 intend to trim the most significant bits, they are used as a
    //       workaround for shiftoverflow errors when size == 8.
    let mask = !(0xFF << size) as u8;
    let flags = ((flags as usize) << size) as u8;

    // if value < 2usize.pow(size) - 1
    if value < (mask as u64) {
        buf.write(flags | value as u8);
        return;
    }

    buf.write(mask | flags);
    let mut remaining = value - mask as u64;

    while remaining >= 128 {
        let rest = (remaining % 128) as u8;
        buf.write(rest + 128);
        remaining /= 128;
    }
    buf.write(remaining as u8);
}

const MAX_POWER: usize = 9 * 7;

impl From<coding::UnexpectedEnd> for Error {
    fn from(_: coding::UnexpectedEnd) -> Self {
        Error::UnexpectedEnd
    }
}

#[cfg(test)]
mod tests;
