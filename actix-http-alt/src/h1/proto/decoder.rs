use bytes::BytesMut;
use http::{Method, Uri};
use httparse::{Request, Status, EMPTY_HEADER};

use super::error::ProtoError;
use crate::HttpRequest;

const MAX_HEADERS: usize = 96;

pub struct Decoder;

impl Decoder {
    fn decode(buf: &mut BytesMut) -> Result<Option<HttpRequest>, ProtoError> {
        let mut parsed = [EMPTY_HEADER; MAX_HEADERS];

        let mut req = Request::new(&mut parsed);

        match req.parse(buf)? {
            Status::Complete(len) => {
                let method = Method::from_bytes(req.method.unwrap().as_bytes())?;
                let uri = req.path.unwrap().parse::<Uri>()?;
                let version = req.version.unwrap();

                Ok(None)
            }
            Status::Partial => Ok(None),
        }
    }
}
