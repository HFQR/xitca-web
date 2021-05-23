use bytes::BytesMut;
use http::{
    header::{HeaderMap, HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, EXPECT, TRANSFER_ENCODING, UPGRADE},
    Method, Uri, Version,
};
use httparse::{Header, Request, Status, EMPTY_HEADER};

use super::error::{Parse, ProtoError};

pub struct Context {
    flag: ContextFlag,
    ctype: ConnectionType,
    header_cache: Vec<HeaderMap>,
}

impl Context {
    const MAX_HEADERS: usize = 96;

    fn new() -> Self {
        let flag = ContextFlag::new(false);

        Self {
            flag,
            ctype: ConnectionType::Close,
            header_cache: Vec::new(),
        }
    }

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Request>, ProtoError> {
        let mut headers = [EMPTY_HEADER; Self::MAX_HEADERS];

        let mut req = Request::new(&mut headers);

        match req.parse(buf)? {
            Status::Complete(len) => {
                let method = Method::from_bytes(req.method.unwrap().as_bytes())?;
                let uri = req.path.unwrap().parse::<Uri>()?;

                // Set connection type to keep alive when given a http/1.1 version.
                let version = if req.version.unwrap() == 1 {
                    self.ctype = if self.flag.keep_alive_enable() {
                        ConnectionType::KeepAlive
                    } else {
                        ConnectionType::Close
                    };

                    Version::HTTP_11
                } else {
                    self.ctype = ConnectionType::Close;
                    Version::HTTP_10
                };

                let mut header_idx = [HeaderIndex::new(); Self::MAX_HEADERS];

                HeaderIndex::record(buf, req.headers, &mut header_idx);

                let headers_len = req.headers.len();

                let slice = buf.split_to(len).freeze();

                let mut headers = self.header_cache.pop().unwrap_or_else(HeaderMap::new);
                headers.reserve(headers_len);

                let mut chunked = false;
                let mut expected = false;
                let mut upgrade = false;
                let mut content_length = None;

                for idx in &header_idx[..headers_len] {
                    let name = HeaderName::from_bytes(&slice[idx.name.0..idx.name.1]).unwrap();
                    let value = HeaderValue::from_maybe_shared(slice.slice(idx.value.0..idx.value.1)).unwrap();

                    match name {
                        TRANSFER_ENCODING => {
                            if version != Version::HTTP_11 {
                                return Err(ProtoError::Parse(Parse::Header));
                            }

                            chunked = value
                                .to_str()
                                .map_err(|_| ProtoError::Parse(Parse::Header))?
                                .trim()
                                .eq_ignore_ascii_case("chunked");
                        }
                        CONTENT_LENGTH => {
                            let len = value
                                .to_str()
                                .map_err(|_| ProtoError::Parse(Parse::Header))?
                                .parse::<u64>()
                                .map_err(|_| ProtoError::Parse(Parse::Header))?;

                            if len != 0 {
                                content_length = Some(len);
                            }
                        }
                        CONNECTION => {
                            if let Ok(value) = value.to_str().map(|conn| conn.trim()) {
                                if value.eq_ignore_ascii_case("keep-alive") && self.flag.keep_alive_enable() {
                                    self.ctype = ConnectionType::KeepAlive;
                                } else if value.eq_ignore_ascii_case("close") {
                                    self.ctype = ConnectionType::Close;
                                } else if value.eq_ignore_ascii_case("upgrade") {
                                    self.ctype = ConnectionType::Upgrade
                                }
                            }
                        }
                        EXPECT => {
                            expected = value.as_bytes() == b"100-continue";
                        }
                        UPGRADE => {
                            // Upgrades are only allowed with HTTP/1.1
                            upgrade = version == Version::HTTP_11;
                        }
                        _ => {}
                    }

                    headers.append(name, value);
                }

                Ok(None)
            }
            Status::Partial => Ok(None),
        }
    }
}

struct ContextFlag {
    flag: u8,
}

impl ContextFlag {
    const ENABLE_KEEP_ALIVE: u8 = 0b0_01;

    fn new(enable_ka: bool) -> Self {
        let flag = if enable_ka { Self::ENABLE_KEEP_ALIVE } else { 0 };

        Self { flag }
    }

    fn keep_alive_enable(&self) -> bool {
        self.flag & Self::ENABLE_KEEP_ALIVE == Self::ENABLE_KEEP_ALIVE
    }
}

#[derive(Clone, Copy)]
pub(crate) struct HeaderIndex {
    pub(crate) name: (usize, usize),
    pub(crate) value: (usize, usize),
}

impl HeaderIndex {
    fn new() -> Self {
        Self {
            name: (0, 0),
            value: (0, 0),
        }
    }

    pub(crate) fn record(bytes: &[u8], headers: &[Header<'_>], indices: &mut [HeaderIndex]) {
        let bytes_ptr = bytes.as_ptr() as usize;
        for (header, indices) in headers.iter().zip(indices.iter_mut()) {
            let name_start = header.name.as_ptr() as usize - bytes_ptr;
            let name_end = name_start + header.name.len();
            indices.name = (name_start, name_end);
            let value_start = header.value.as_ptr() as usize - bytes_ptr;
            let value_end = value_start + header.value.len();
            indices.value = (value_start, value_end);
        }
    }
}

/// Represents various types of connection
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ConnectionType {
    /// Close connection after response
    Close,

    /// Keep connection alive after response
    KeepAlive,

    /// Connection is upgraded to different type
    Upgrade,
}
