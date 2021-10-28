use httparse::{Status, EMPTY_HEADER};

use xitca_http::{
    bytes::BytesMut,
    h1::proto::{
        codec::TransferCoding,
        context::ConnectionType,
        error::{Parse, ProtoError},
        header::HeaderIndex,
    },
    http::{
        header::{HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, TRANSFER_ENCODING, UPGRADE},
        Response, StatusCode, Version,
    },
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
                                    self.set_ctype(ConnectionType::KeepAlive);
                                } else if value.eq_ignore_ascii_case("close") {
                                    self.set_ctype(ConnectionType::Close);
                                } else if value.eq_ignore_ascii_case("upgrade") {
                                    // set decoder to upgrade variant.
                                    decoder.try_set(TransferCoding::plain_chunked())?;
                                    self.set_ctype(ConnectionType::Upgrade);
                                }
                            }
                        }
                        // Upgrades are only allowed with HTTP/1.1
                        UPGRADE if version == Version::HTTP_11 => self.set_ctype(ConnectionType::Upgrade),
                        _ => {}
                    }

                    headers.append(name, value);
                }

                let mut res = Response::new(());

                *res.version_mut() = version;
                *res.status_mut() = status;
                *res.headers_mut() = headers;

                Ok(Some((res, decoder)))
            }
            Status::Partial => Ok(None),
        }
    }
}
