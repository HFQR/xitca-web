use std::mem;

use httparse::Status;

use crate::{
    bytes::{Buf, Bytes, BytesMut},
    http::{
        header::{HeaderMap, HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, EXPECT, TRANSFER_ENCODING, UPGRADE},
        Method, Request, Uri, Version,
    },
};

use super::{
    codec::TransferCoding,
    context::{ConnectionType, Context},
    error::{Parse, ProtoError},
    header::HeaderIndex,
};

impl<D, const MAX_HEADERS: usize> Context<'_, D, MAX_HEADERS> {
    // decode head and generate request and body decoder.
    pub(super) fn decode_head<const READ_BUF_LIMIT: usize>(
        &mut self,
        buf: &mut BytesMut,
    ) -> Result<Option<(Request<()>, TransferCoding)>, ProtoError> {
        let mut req = httparse::Request::new(&mut []);
        let mut headers = [mem::MaybeUninit::uninit(); MAX_HEADERS];

        match req.parse_with_uninit_headers(buf, &mut headers)? {
            Status::Complete(len) => {
                // Important: reset context state for new request.
                self.reset();

                let method = Method::from_bytes(req.method.unwrap().as_bytes())?;

                let uri = req.path.unwrap().parse::<Uri>()?;

                // Set connection type when doing version match.
                let version = if req.version.unwrap() == 1 {
                    // Default ctype is KeepAlive so set_ctype is skipped here.
                    Version::HTTP_11
                } else {
                    self.set_ctype(ConnectionType::Close);
                    Version::HTTP_10
                };

                // record the index of headers from the buffer.
                let mut header_idx = HeaderIndex::new_array::<MAX_HEADERS>();
                HeaderIndex::record(buf, req.headers, &mut header_idx);

                let headers_len = req.headers.len();

                // split the headers from buffer.
                let slice = buf.split_to(len).freeze();

                // default decoder.
                let mut decoder = TransferCoding::eof();

                // pop a cached headermap or construct a new one.
                let mut headers = self.take_headers();
                headers.reserve(headers_len);

                // write headers to headermap and update request states.
                header_idx
                    .iter()
                    .take(headers_len)
                    .try_for_each(|idx| self.try_write_header(&mut headers, &mut decoder, idx, &slice, version))?;

                if method == Method::CONNECT {
                    self.set_ctype(ConnectionType::Upgrade);
                    // set method to context so it can pass method to response.
                    self.set_connect_method();
                    decoder.try_set(TransferCoding::upgrade())?;
                }

                let mut req = Request::new(());

                let extensions = self.take_extensions();

                *req.method_mut() = method;
                *req.version_mut() = version;
                *req.uri_mut() = uri;
                *req.headers_mut() = headers;
                *req.extensions_mut() = extensions;

                Ok(Some((req, decoder)))
            }

            Status::Partial => {
                if buf.remaining() >= READ_BUF_LIMIT {
                    Err(ProtoError::Parse(Parse::HeaderTooLarge))
                } else {
                    Ok(None)
                }
            }
        }
    }

    pub fn try_write_header(
        &mut self,
        headers: &mut HeaderMap,
        decoder: &mut TransferCoding,
        idx: &HeaderIndex,
        slice: &Bytes,
        version: Version,
    ) -> Result<(), ProtoError> {
        let name = HeaderName::from_bytes(&slice[idx.name.0..idx.name.1]).unwrap();
        let value = HeaderValue::from_maybe_shared(slice.slice(idx.value.0..idx.value.1)).unwrap();

        match name {
            TRANSFER_ENCODING => {
                if version != Version::HTTP_11 {
                    return Err(ProtoError::Parse(Parse::HeaderName));
                }

                let chunked = value
                    .to_str()
                    .map_err(|_| ProtoError::Parse(Parse::HeaderName))?
                    .rsplit(',')
                    .next()
                    .map(|v| v.trim().eq_ignore_ascii_case("chunked"))
                    .unwrap_or(false);

                if chunked {
                    decoder.try_set(TransferCoding::decode_chunked())?;
                } else {
                    return Err(ProtoError::Parse(Parse::HeaderName));
                }
            }
            CONTENT_LENGTH => {
                let len = value
                    .to_str()
                    .map_err(|_| ProtoError::Parse(Parse::HeaderName))?
                    .parse::<u64>()
                    .map_err(|_| ProtoError::Parse(Parse::HeaderName))?;

                if len != 0 {
                    decoder.try_set(TransferCoding::length(len))?;
                }
            }
            CONNECTION => {
                let v = value.as_bytes();
                // Connection header would update context state.
                if v.eq_ignore_ascii_case(b"keep-alive") {
                    self.set_ctype(ConnectionType::KeepAlive);
                } else if v.eq_ignore_ascii_case(b"close") {
                    self.set_ctype(ConnectionType::Close);
                } else if v.eq_ignore_ascii_case(b"upgrade") {
                    // set decoder to upgrade variant.
                    decoder.try_set(TransferCoding::upgrade())?;
                    self.set_ctype(ConnectionType::Upgrade);
                }
            }
            EXPECT if value.as_bytes() == b"100-continue" => self.set_expect_header(),
            // Upgrades are only allowed with HTTP/1.1
            UPGRADE if version == Version::HTTP_11 => self.set_ctype(ConnectionType::Upgrade),
            _ => {}
        }

        headers.append(name, value);

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use tokio::task::LocalSet;

    #[tokio::test]
    async fn transfer_encoding() {
        LocalSet::new()
            .run_until(async {
                let date = crate::date::DateTimeService::new();
                let mut ctx = Context::<_, 4>::new(date.get());

                let mut buf = BytesMut::new();

                let head = b"\
                GET / HTTP/1.1\r\n\
                Transfer-Encoding: gzip, chunked\r\n\
                \r\n\
                ";

                buf.extend_from_slice(head);

                let (req, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
                assert_eq!(
                    req.headers().get(TRANSFER_ENCODING).unwrap().to_str().unwrap(),
                    "gzip, chunked"
                );

                match decoder {
                    TransferCoding::DecodeChunked(_, _) => {}
                    _ => panic!("trasnfer coding is not decoded chunked"),
                };
            })
            .await
    }
}
