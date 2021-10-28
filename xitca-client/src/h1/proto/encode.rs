use xitca_http::{
    bytes::{BufMut, BytesMut},
    h1::proto::{codec::TransferCoding, error::ProtoError},
    http::{request::Parts, version::Version},
};

use crate::body::RequestBodySize;

use super::context::Context;

impl<const HEADER_LIMIT: usize> Context<'_, '_, HEADER_LIMIT> {
    pub(super) fn encode_head(
        &mut self,
        buf: &mut BytesMut,
        parts: Parts,
        size: RequestBodySize,
    ) -> Result<TransferCoding, ProtoError> {
        let method = parts.method;
        let uri = parts.uri;
        let headers = parts.headers;
        let extensions = parts.extensions;
        let version = parts.version;

        // encode line of "Method PathQuery Version"
        let method = method.as_str().as_bytes();
        let path_and_query = uri.path_and_query().map(|u| u.as_str()).unwrap_or("/").as_bytes();
        let version = match version {
            Version::HTTP_09 => b" HTTP/0.9\r\n",
            Version::HTTP_10 => b" HTTP/1.0\r\n",
            Version::HTTP_11 => b" HTTP/1.1\r\n",
            Version::HTTP_2 => b" HTTP/2.0\r\n",
            Version::HTTP_3 => b" HTTP/3.0\r\n",
            _ => todo!("handle error"),
        };

        buf.reserve(method.len() + 1 + path_and_query.len() + 11);

        buf.put_slice(method);
        buf.put_slice(b" ");
        buf.put_slice(path_and_query);
        buf.put_slice(version);

        self.encode_headers(headers, extensions, size, buf, false)
    }
}
