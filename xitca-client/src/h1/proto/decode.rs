use std::mem;

use bytes::BytesMut;
use httparse::{Header, Status, EMPTY_HEADER};

use crate::http::{
    header::{HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, EXPECT, TRANSFER_ENCODING, UPGRADE},
    Response, StatusCode, Version,
};

use super::{
    context::Context,
    error::{Parse, ProtoError},
};

const MAX_HEADERS: usize = 128;

impl Context {
    pub fn decode(&mut self, buf: &mut BytesMut) -> Result<Response<()>, ProtoError> {
        let mut headers = [EMPTY_HEADER; MAX_HEADERS];

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
                let mut header_idx = [HeaderIndex::new(); MAX_HEADERS];
                HeaderIndex::record(buf, parsed.headers, &mut header_idx);

                let headers_len = parsed.headers.len();

                // split the headers from buffer.
                let slice = buf.split_to(len).freeze();

                let mut headers = mem::take(self.headers_mut());
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

                            // if chunked {
                            //     decoder.try_set(TransferCoding::decode_chunked())?;
                            // }
                        }
                        CONTENT_LENGTH => {
                            let len = value
                                .to_str()
                                .map_err(|_| ProtoError::Parse(Parse::HeaderName))?
                                .parse::<u64>()
                                .map_err(|_| ProtoError::Parse(Parse::HeaderName))?;

                            // if len != 0 {
                            //     decoder.try_set(TransferCoding::length(len))?;
                            // }
                        }
                        CONNECTION => {
                            if let Ok(value) = value.to_str().map(|conn| conn.trim()) {
                                // Connection header would update context state.
                                // if value.eq_ignore_ascii_case("keep-alive") {
                                //     self.set_ctype(ConnectionType::KeepAlive);
                                // } else if value.eq_ignore_ascii_case("close") {
                                //     self.set_ctype(ConnectionType::Close);
                                // } else if value.eq_ignore_ascii_case("upgrade") {
                                //     // set decoder to upgrade variant.
                                //     decoder.try_set(TransferCoding::plain_chunked())?;
                                //     self.set_ctype(ConnectionType::Upgrade);
                                // }
                            }
                        }
                        // EXPECT if value.as_bytes() == b"100-continue" => self.set_expect_header(),
                        // // Upgrades are only allowed with HTTP/1.1
                        // UPGRADE if version == Version::HTTP_11 => self.set_ctype(ConnectionType::Upgrade),
                        _ => {}
                    }

                    headers.append(name, value);
                }

                let mut res = Response::new(());

                *res.version_mut() = version;
                *res.status_mut() = status;
                // *res.headers_mut() = headers;

                Ok(res)
            }
            Status::Partial => todo!(),
        }
    }
}

#[derive(Clone, Copy)]
struct HeaderIndex {
    name: (usize, usize),
    value: (usize, usize),
}

impl HeaderIndex {
    const fn new() -> Self {
        Self {
            name: (0, 0),
            value: (0, 0),
        }
    }

    fn record(bytes: &[u8], headers: &[Header<'_>], indices: &mut [Self]) {
        let bytes_ptr = bytes.as_ptr() as usize;

        headers.iter().zip(indices.iter_mut()).for_each(|(header, indice)| {
            let name_start = header.name.as_ptr() as usize - bytes_ptr;
            let value_start = header.value.as_ptr() as usize - bytes_ptr;

            let name_end = name_start + header.name.len();
            let value_end = value_start + header.value.len();

            indice.name = (name_start, name_end);
            indice.value = (value_start, value_end);
        });
    }
}
