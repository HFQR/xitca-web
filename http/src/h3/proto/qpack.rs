pub use self::{
    decoder::{Decoded, DecoderError, decode_stateless},
    encoder::{EncoderError, encode_stateless},
    field::HeaderField,
};

mod block;
mod decoder;
mod dynamic;
mod encoder;
mod field;
mod parse_error;
mod prefix_int;
mod prefix_string;
mod static_;
mod stream;
mod vas;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub enum Error {
    Encoder(EncoderError),
    Decoder(DecoderError),
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Encoder(e) => write!(f, "Encoder {}", e),
            Error::Decoder(e) => write!(f, "Decoder {}", e),
        }
    }
}
