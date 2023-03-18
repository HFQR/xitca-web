use postgres_protocol::message::backend;
use xitca_io::bytes::BytesMut;

use crate::error::Error;

pub(super) fn decode(msg: &mut BytesMut) -> Result<Option<backend::Message>, Error> {
    match backend::Message::parse(msg)? {
        // TODO: error response.
        Some(backend::Message::ErrorResponse(_body)) => Err(Error::ToDo),
        Some(msg) => Ok(Some(msg)),
        None => Ok(None),
    }
}
