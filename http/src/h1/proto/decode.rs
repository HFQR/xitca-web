use httparse::Status;
use xitca_unsafe_collection::uninit;

use crate::{
    bytes::{Buf, Bytes},
    http::{
        header::{HeaderMap, HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, EXPECT, TRANSFER_ENCODING, UPGRADE},
        Extension, Method, Request, RequestExt, Uri, Version,
    },
    util::buffered::PagedBytesMut,
};

use super::{
    codec::TransferCoding,
    context::{ConnectionType, Context},
    error::{Parse, ProtoError},
    header::HeaderIndex,
};

type Decoded = (Request<RequestExt<()>>, TransferCoding);

impl<D, const MAX_HEADERS: usize> Context<'_, D, MAX_HEADERS> {
    // decode head and generate request and body decoder.
    pub fn decode_head<const READ_BUF_LIMIT: usize>(
        &mut self,
        buf: &mut PagedBytesMut,
    ) -> Result<Option<Decoded>, ProtoError> {
        let mut req = httparse::Request::new(&mut []);
        let mut headers = uninit::uninit_array::<_, MAX_HEADERS>();

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

                // record indices of headers from bytes buffer.
                let mut header_idx = uninit::uninit_array::<_, MAX_HEADERS>();
                let header_idx_slice = HeaderIndex::record(&mut header_idx, buf, req.headers);

                let headers_len = req.headers.len();

                // split the headers from buffer.
                let slice = buf.split_to(len).freeze();

                // default decoder.
                let mut decoder = TransferCoding::eof();

                // pop a cached headermap or construct a new one.
                let mut headers = self.take_headers();
                headers.reserve(headers_len);

                // write headers to headermap and update request states.
                header_idx_slice
                    .iter()
                    .try_for_each(|idx| self.try_write_header(&mut headers, &mut decoder, idx, &slice, version))?;

                // set method to context so it can pass method to response.
                match method {
                    Method::CONNECT => {
                        self.set_connect_method();
                        decoder.try_set(TransferCoding::upgrade())?;
                    }
                    Method::HEAD => self.set_head_method(),
                    _ => {}
                }

                let ext = Extension::new(*self.socket_addr());
                let mut req = Request::new(RequestExt::from_parts((), ext));

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
                    .map_err(|_| Parse::HeaderName)?
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
                    .map_err(|_| Parse::HeaderValue)?
                    .parse::<u64>()
                    .map_err(|_| Parse::HeaderValue)?;

                if len != 0 {
                    decoder.try_set(TransferCoding::length(len))?;
                }
            }
            CONNECTION => self.try_set_ctype_from_header(&value)?,
            EXPECT => {
                if value.as_bytes() != b"100-continue" {
                    return Err(ProtoError::Parse(Parse::HeaderValue));
                }
                self.set_expect_header()
            }
            UPGRADE => {
                if version != Version::HTTP_11 {
                    return Err(ProtoError::Parse(Parse::HeaderName));
                }
                decoder.try_set(TransferCoding::upgrade())?;
            }
            _ => {}
        }

        headers.append(name, value);

        Ok(())
    }

    pub(super) fn try_set_ctype_from_header(&mut self, val: &HeaderValue) -> Result<(), ProtoError> {
        for val in val.to_str().map_err(|_| Parse::HeaderValue)?.split(',') {
            let val = val.trim();
            if val.eq_ignore_ascii_case("keep-alive") {
                self.set_ctype(ConnectionType::KeepAlive)
            } else if val.eq_ignore_ascii_case("close") {
                self.set_ctype(ConnectionType::Close);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn connection_multiple_value() {
        let mut ctx = Context::<_, 4>::new(&());

        let head = b"\
                GET / HTTP/1.1\r\n\
                Connection: keep-alive, upgrade\r\n\
                \r\n\
                ";
        let mut buf = PagedBytesMut::from(&head[..]);

        let _ = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        assert!(!ctx.is_connection_closed());

        // this is a wrong connection header but instead of rejecting it the last close value is
        // used for the final value. there is no particular reason behind this behaviour and this
        // session of the test serves only as a consistency check to prevent regression.
        let head = b"\
                GET / HTTP/1.1\r\n\
                Connection: keep-alive, close, upgrade\r\n\
                \r\n\
                ";
        let mut buf = PagedBytesMut::from(&head[..]);

        let _ = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        assert!(ctx.is_connection_closed());

        let head = b"\
                GET / HTTP/1.1\r\n\
                Connection: close, keep-alive, upgrade\r\n\
                \r\n\
                ";
        let mut buf = PagedBytesMut::from(&head[..]);

        let _ = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        assert!(!ctx.is_connection_closed());
    }

    #[test]
    fn transfer_encoding() {
        let mut ctx = Context::<_, 4>::new(&());

        let head = b"\
                GET / HTTP/1.1\r\n\
                Transfer-Encoding: gzip, chunked\r\n\
                \r\n\
                ";
        let mut buf = PagedBytesMut::from(&head[..]);

        let (req, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        assert_eq!(
            req.headers().get(TRANSFER_ENCODING).unwrap().to_str().unwrap(),
            "gzip, chunked"
        );

        assert!(
            matches!(decoder, TransferCoding::DecodeChunked(..)),
            "transfer coding is not decoded to chunked"
        );

        ctx.reset();

        let head = b"\
                GET / HTTP/1.1\r\n\
                Transfer-Encoding: chunked\r\n\
                \r\n\
                ";
        let mut buf = PagedBytesMut::from(&head[..]);

        let (req, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        assert_eq!(
            req.headers().get(TRANSFER_ENCODING).unwrap().to_str().unwrap(),
            "chunked"
        );

        assert!(
            matches!(decoder, TransferCoding::DecodeChunked(..)),
            "transfer coding is not decoded to chunked"
        );

        let head = b"\
                GET / HTTP/1.1\r\n\
                Transfer-Encoding: identity\r\n\
                \r\n\
                ";
        let mut buf = PagedBytesMut::from(&head[..]);

        assert!(ctx.decode_head::<128>(&mut buf).is_err());

        let head = b"\
                GET / HTTP/1.1\r\n\
                Transfer-Encoding: chunked, gzip\r\n\
                \r\n\
                ";
        let mut buf = PagedBytesMut::from(&head[..]);

        assert!(ctx.decode_head::<128>(&mut buf).is_err());
    }
}
