use xitca_http::{
    body::Body,
    h1::proto::{error::ProtoError, trasnder_coding::TransferCoding},
};

use crate::{
    body::BodySize,
    bytes::{BufMut, BytesMut},
    http::{request::Request, version::Version},
};

use super::context::Context;

impl<const HEADER_LIMIT: usize> Context<'_, '_, HEADER_LIMIT> {
    pub(super) fn encode_head<B>(
        &mut self,
        buf: &mut BytesMut,
        req: &mut Request<B>,
    ) -> Result<TransferCoding, ProtoError>
    where
        B: Body,
    {
        let method = req.method();
        let uri = req.uri();
        let version = req.version();

        // encode line of "Method PathQuery Version"
        let method = method.as_str().as_bytes();
        let path_and_query = uri.path_and_query().map(|u| u.as_str()).unwrap_or("/").as_bytes();
        let version = match version {
            Version::HTTP_09 => b" HTTP/0.9",
            Version::HTTP_10 => b" HTTP/1.0",
            Version::HTTP_11 => b" HTTP/1.1",
            Version::HTTP_2 => b" HTTP/2.0",
            Version::HTTP_3 => b" HTTP/3.0",
            _ => todo!("handle error"),
        };

        buf.reserve(method.len() + 1 + path_and_query.len() + 9);

        buf.put_slice(method);
        buf.put_slice(b" ");
        buf.put_slice(path_and_query);
        buf.put_slice(version);

        let size = BodySize::from_body(req.body());

        let headers = req.headers_mut();

        self.encode_headers(headers, size, buf, false)
    }
}
