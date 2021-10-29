use httparse::{Status, EMPTY_HEADER};

use xitca_http::{
    bytes::BytesMut,
    h1::proto::{
        codec::TransferCoding,
        error::{Parse, ProtoError},
        header::HeaderIndex,
    },
    http::{Response, StatusCode, Version},
};

use super::context::Context;

impl<const HEADER_LIMIT: usize> Context<'_, '_, HEADER_LIMIT> {
    pub(crate) fn decode_head(
        &mut self,
        buf: &mut BytesMut,
    ) -> Result<Option<(Response<()>, TransferCoding)>, ProtoError> {
        let mut headers = [EMPTY_HEADER; HEADER_LIMIT];

        let mut parsed = httparse::Response::new(&mut headers);

        match parsed.parse(buf.as_ref())? {
            Status::Complete(len) => {
                let version = if parsed.version.unwrap() == 1 {
                    Version::HTTP_11
                } else {
                    Version::HTTP_10
                };

                let status = StatusCode::from_u16(parsed.code.unwrap()).map_err(|_| Parse::StatusCode)?;

                // record the index of headers from the buffer.
                let mut header_idx = HeaderIndex::new_array::<HEADER_LIMIT>();
                HeaderIndex::record(buf, parsed.headers, &mut header_idx);

                let headers_len = parsed.headers.len();

                // split the headers from buffer.
                let slice = buf.split_to(len).freeze();

                let mut headers = self.take_headers();
                headers.reserve(headers_len);

                let mut decoder = TransferCoding::eof();

                // write headers to headermap and update request states.
                header_idx
                    .iter()
                    .take(headers_len)
                    .try_for_each(|idx| self.try_write_header(&mut headers, &mut decoder, idx, &slice, version))?;

                let extensions = self.take_extensions();

                let mut res = Response::new(());
                *res.version_mut() = version;
                *res.status_mut() = status;
                *res.headers_mut() = headers;
                *res.extensions_mut() = extensions;

                Ok(Some((res, decoder)))
            }
            Status::Partial => Ok(None),
        }
    }
}
