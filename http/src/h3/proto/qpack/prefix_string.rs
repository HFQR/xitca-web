mod bitwin;
mod decode;
mod encode;

use std::convert::TryInto;
use std::fmt;
use std::num::TryFromIntError;

use crate::bytes::{Buf, BufMut};

pub use self::bitwin::BitWindow;

pub use self::{
    decode::{Error as HuffmanDecodingError, HpackStringDecode},
    encode::{Error as HuffmanEncodingError, HpackStringEncode},
};

use super::prefix_int::{self, Error as IntegerError};
use crate::h3::proto::coding::BufMutExt;

#[derive(Debug, PartialEq)]
pub enum Error {
    UnexpectedEnd,
    Integer(IntegerError),
    HuffmanDecoding(HuffmanDecodingError),
    HuffmanEncoding(HuffmanEncodingError),
    BufSize(TryFromIntError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnexpectedEnd => write!(f, "unexpected end"),
            Error::Integer(e) => write!(f, "could not parse integer: {}", e),
            Error::HuffmanDecoding(e) => write!(f, "Huffman decode failed: {:?}", e),
            Error::HuffmanEncoding(e) => write!(f, "Huffman encode failed: {:?}", e),
            Error::BufSize(_) => write!(f, "number in buffer wrong size"),
        }
    }
}

pub fn decode<B: Buf>(size: u8, buf: &mut B) -> Result<Vec<u8>, Error> {
    let (flags, len) = prefix_int::decode(size - 1, buf)?;
    let len: usize = len.try_into()?;
    if buf.remaining() < len {
        return Err(Error::UnexpectedEnd);
    }

    let payload = buf.copy_to_bytes(len);
    let value = if flags & 1 == 0 {
        payload.into_iter().collect()
    } else {
        let mut decoded = Vec::new();
        for byte in payload.into_iter().collect::<Vec<u8>>().hpack_decode() {
            decoded.push(byte?);
        }
        decoded
    };
    Ok(value)
}

pub fn encode<B: BufMut>(size: u8, flags: u8, value: &[u8], buf: &mut B) -> Result<(), Error> {
    let encoded = Vec::from(value).hpack_encode()?;
    prefix_int::encode(size - 1, flags << 1 | 1, encoded.len().try_into()?, buf);
    for byte in encoded {
        buf.write(byte);
    }
    Ok(())
}

impl From<HuffmanEncodingError> for Error {
    fn from(error: HuffmanEncodingError) -> Self {
        Error::HuffmanEncoding(error)
    }
}

impl From<IntegerError> for Error {
    fn from(error: IntegerError) -> Self {
        match error {
            IntegerError::UnexpectedEnd => Error::UnexpectedEnd,
            e => Error::Integer(e),
        }
    }
}

impl From<HuffmanDecodingError> for Error {
    fn from(error: HuffmanDecodingError) -> Self {
        Error::HuffmanDecoding(error)
    }
}

impl From<TryFromIntError> for Error {
    fn from(error: TryFromIntError) -> Self {
        Error::BufSize(error)
    }
}

#[cfg(test)]
mod tests;
