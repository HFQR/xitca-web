use std::io;

use super::hpack::DecoderError;

#[derive(Debug)]
pub(super) enum Error {
    MalformedMessage,
    Hpack(DecoderError),
    Io(io::Error),
}

impl From<DecoderError> for Error {
    fn from(e: DecoderError) -> Self {
        Self::Hpack(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
