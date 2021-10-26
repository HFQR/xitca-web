use std::mem;

use bytes::{Buf, BytesMut};
use httparse::Status;

use crate::http::{
    header::{HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, EXPECT, TRANSFER_ENCODING, UPGRADE},
    Method, Request, Uri, Version,
};

use super::{
    codec::TransferCoding,
    context::{ConnectionType, ServerContext},
    error::{Parse, ProtoError},
    header::HeaderIndex,
};

impl<const MAX_HEADERS: usize> ServerContext<'_, MAX_HEADERS> {
    // decode head and generate request and body decoder.
    pub(super) fn decode_head<const READ_BUF_LIMIT: usize>(
        &mut self,
        buf: &mut BytesMut,
    ) -> Result<Option<(Request<()>, TransferCoding)>, ProtoError> {
        let mut req = httparse::Request::new(&mut []);
        let mut headers = [mem::MaybeUninit::uninit(); MAX_HEADERS];

        match req.parse_with_uninit_headers(buf, &mut headers)? {
            Status::Complete(len) => {
                let ctx = self.ctx();

                // Important: reset context state for new request.
                ctx.reset();

                let method = Method::from_bytes(req.method.unwrap().as_bytes())?;

                let uri = req.path.unwrap().parse::<Uri>()?;

                // Set connection type when doing version match.
                let version = if req.version.unwrap() == 1 {
                    // Default ctype is KeepAlive so set_ctype is skipped here.
                    Version::HTTP_11
                } else {
                    ctx.set_ctype(ConnectionType::Close);
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
                let mut headers = ctx.take_headers();
                headers.reserve(headers_len);

                // write headers to headermap and update request states.
                for idx in &header_idx[..headers_len] {
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
                                .trim()
                                .eq_ignore_ascii_case("chunked");

                            if chunked {
                                decoder.try_set(TransferCoding::decode_chunked())?;
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
                            if let Ok(value) = value.to_str().map(|conn| conn.trim()) {
                                // Connection header would update context state.
                                if value.eq_ignore_ascii_case("keep-alive") {
                                    ctx.set_ctype(ConnectionType::KeepAlive);
                                } else if value.eq_ignore_ascii_case("close") {
                                    ctx.set_ctype(ConnectionType::Close);
                                } else if value.eq_ignore_ascii_case("upgrade") {
                                    // set decoder to upgrade variant.
                                    decoder.try_set(TransferCoding::plain_chunked())?;
                                    ctx.set_ctype(ConnectionType::Upgrade);
                                }
                            }
                        }
                        EXPECT if value.as_bytes() == b"100-continue" => ctx.set_expect_header(),
                        // Upgrades are only allowed with HTTP/1.1
                        UPGRADE if version == Version::HTTP_11 => ctx.set_ctype(ConnectionType::Upgrade),
                        _ => {}
                    }

                    headers.append(name, value);
                }

                if method == Method::CONNECT {
                    ctx.set_ctype(ConnectionType::Upgrade);
                    // set method to context so it can pass method to response.
                    ctx.set_connect_method();
                    decoder.try_set(TransferCoding::plain_chunked())?;
                }

                let mut req = Request::new(());

                let extensions = ctx.take_extensions();

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
}
