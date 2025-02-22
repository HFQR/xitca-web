use core::mem::MaybeUninit;

use http::uri::{Authority, Scheme};
use httparse::Status;

use crate::{
    bytes::{Buf, Bytes, BytesMut},
    http::{
        Extension, Method, Request, RequestExt, Uri, Version,
        header::{CONNECTION, CONTENT_LENGTH, EXPECT, HeaderMap, HeaderName, HeaderValue, TRANSFER_ENCODING, UPGRADE},
    },
};

use super::{
    codec::TransferCoding,
    context::Context,
    error::ProtoError,
    header::{self, HeaderIndex},
};

type Decoded = (Request<RequestExt<()>>, TransferCoding);

impl<D, const MAX_HEADERS: usize> Context<'_, D, MAX_HEADERS> {
    // decode head and generate request and body decoder.
    pub fn decode_head<const READ_BUF_LIMIT: usize>(
        &mut self,
        buf: &mut BytesMut,
    ) -> Result<Option<Decoded>, ProtoError> {
        let mut req = httparse::Request::new(&mut []);
        let mut headers = [const { MaybeUninit::uninit() }; MAX_HEADERS];

        match req.parse_with_uninit_headers(buf, &mut headers)? {
            Status::Complete(len) => {
                // Important: reset context state for new request.
                self.reset();

                let method = Method::from_bytes(req.method.unwrap().as_bytes())?;

                // default body decoder from method.
                let mut decoder = match method {
                    // set method to context so it can pass method to response.
                    Method::CONNECT => {
                        self.set_connect_method();
                        TransferCoding::upgrade()
                    }
                    Method::HEAD => {
                        self.set_head_method();
                        TransferCoding::eof()
                    }
                    _ => TransferCoding::eof(),
                };

                // Set connection type when doing version match.
                let version = if req.version.unwrap() == 1 {
                    // Default ctype is KeepAlive so set_ctype is skipped here.
                    Version::HTTP_11
                } else {
                    self.set_close();
                    Version::HTTP_10
                };

                // record indices of headers from bytes buffer.
                let mut header_idx = [const { MaybeUninit::uninit() }; MAX_HEADERS];
                let header_idx_slice = HeaderIndex::record(&mut header_idx, buf, req.headers);
                let headers_len = req.headers.len();

                // record indices of request path from buffer.
                let path = req.path.unwrap();
                let path_head = path.as_ptr() as usize - buf.as_ptr() as usize;
                let path_len = path.len();

                // split the headers from buffer.
                let slice = buf.split_to(len).freeze();

                let mut uri = Uri::from_maybe_shared(slice.slice(path_head..path_head + path_len))?.into_parts();

                // pop a cached headermap or construct a new one.
                let mut headers = self.take_headers();
                headers.reserve(headers_len);

                // write headers to headermap and update request states.
                for idx in header_idx_slice {
                    self.try_write_header(&mut headers, &mut decoder, idx, &slice, version)?;
                }

                let ext = Extension::new(*self.socket_addr());
                let mut req = Request::new(RequestExt::from_parts((), ext));

                let extensions = self.take_extensions();

                // Try to set authority from host header if not present in request path
                if uri.authority.is_none() {
                    // @TODO if it's a tls connection we could set the sni server name as authority instead
                    if let Some(host) = headers.get(http::header::HOST) {
                        uri.authority = Some(Authority::try_from(host.as_bytes())?);
                    }
                }

                // If authority is set, this will set the correct scheme depending on the tls acceptor used in the service.
                if uri.authority.is_some() && uri.scheme.is_none() {
                    uri.scheme = if self.is_tls {
                        Some(Scheme::HTTPS)
                    } else {
                        Some(Scheme::HTTP)
                    };
                }

                let uri = Uri::from_parts(uri)?;

                *req.method_mut() = method;
                *req.version_mut() = version;
                *req.uri_mut() = uri;
                *req.headers_mut() = headers;
                *req.extensions_mut() = extensions;

                Ok(Some((req, decoder)))
            }

            Status::Partial => {
                if buf.remaining() >= READ_BUF_LIMIT {
                    Err(ProtoError::HeaderTooLarge)
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
                    return Err(ProtoError::HeaderName);
                }
                for val in value.to_str().map_err(|_| ProtoError::HeaderValue)?.split(',') {
                    let val = val.trim();
                    if val.eq_ignore_ascii_case("chunked") {
                        decoder.try_set(TransferCoding::decode_chunked())?;
                    }
                }
            }
            CONTENT_LENGTH => {
                let len = header::parse_content_length(&value)?;
                decoder.try_set(TransferCoding::length(len))?;
            }
            CONNECTION => self.try_set_close_from_header(&value)?,
            EXPECT => {
                if !value.as_bytes().eq_ignore_ascii_case(b"100-continue") {
                    return Err(ProtoError::HeaderValue);
                }
                self.set_expect_header()
            }
            UPGRADE => {
                if version != Version::HTTP_11 {
                    return Err(ProtoError::HeaderName);
                }
                decoder.try_set(TransferCoding::upgrade())?;
            }
            _ => {}
        }

        headers.append(name, value);

        Ok(())
    }

    pub(super) fn try_set_close_from_header(&mut self, val: &HeaderValue) -> Result<(), ProtoError> {
        for val in val.to_str().map_err(|_| ProtoError::HeaderValue)?.split(',') {
            let val = val.trim();
            if val.eq_ignore_ascii_case("keep-alive") {
                self.remove_close()
            } else if val.eq_ignore_ascii_case("close") {
                self.set_close()
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
        let mut ctx = Context::<_, 4>::new(&(), false);

        let head = b"\
                GET / HTTP/1.1\r\n\
                Connection: keep-alive, upgrade\r\n\
                \r\n\
                ";
        let mut buf = BytesMut::from(&head[..]);

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
        let mut buf = BytesMut::from(&head[..]);

        let _ = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        assert!(ctx.is_connection_closed());

        let head = b"\
                GET / HTTP/1.1\r\n\
                Connection: close, keep-alive, upgrade\r\n\
                \r\n\
                ";
        let mut buf = BytesMut::from(&head[..]);

        let _ = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        assert!(!ctx.is_connection_closed());
    }

    #[test]
    fn transfer_encoding() {
        let mut ctx = Context::<_, 4>::new(&(), false);

        let head = b"\
                GET / HTTP/1.1\r\n\
                Transfer-Encoding: gzip\r\n\
                Transfer-Encoding: chunked\r\n\
                \r\n\
                ";
        let mut buf = BytesMut::from(&head[..]);

        let (req, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        let mut iter = req.headers().get_all(TRANSFER_ENCODING).into_iter();
        assert_eq!(iter.next().unwrap().to_str().unwrap(), "gzip");
        assert_eq!(iter.next().unwrap().to_str().unwrap(), "chunked");
        assert!(
            matches!(decoder, TransferCoding::DecodeChunked(..)),
            "transfer coding is not decoded to chunked"
        );

        ctx.reset();

        let head = b"\
        GET / HTTP/1.1\r\n\
        Transfer-Encoding: chunked\r\n\
        Transfer-Encoding: gzip\r\n\
        \r\n\
        ";
        let mut buf = BytesMut::from(&head[..]);

        let (req, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        let mut iter = req.headers().get_all(TRANSFER_ENCODING).into_iter();
        assert_eq!(iter.next().unwrap().to_str().unwrap(), "chunked");
        assert_eq!(iter.next().unwrap().to_str().unwrap(), "gzip");
        assert!(
            matches!(decoder, TransferCoding::DecodeChunked(..)),
            "transfer coding is not decoded to chunked"
        );

        ctx.reset();

        let head = b"\
                GET / HTTP/1.1\r\n\
                Transfer-Encoding: gzip, chunked\r\n\
                \r\n\
                ";
        let mut buf = BytesMut::from(&head[..]);

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
        let mut buf = BytesMut::from(&head[..]);

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
        let mut buf = BytesMut::from(&head[..]);
        let (_, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        assert!(
            matches!(decoder, TransferCoding::Eof),
            "transfer coding is not decoded to eof"
        );

        let head = b"\
                GET / HTTP/1.1\r\n\
                Transfer-Encoding: chunked, gzip\r\n\
                \r\n\
                ";
        let mut buf = BytesMut::from(&head[..]);
        let (_, decoder) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();
        assert!(
            matches!(decoder, TransferCoding::DecodeChunked(..)),
            "transfer coding is not decoded to chunked"
        );
    }

    #[test]
    fn test_host_with_scheme() {
        let mut ctx = Context::<_, 4>::new(&(), true);

        let head = b"\
                GET / HTTP/1.1\r\n\
                Host: example.com\r\n\
                \r\n\
                ";
        let mut buf = BytesMut::from(&head[..]);

        let (req, _) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();

        assert_eq!(req.uri().scheme(), Some(&Scheme::HTTPS));
        assert_eq!(req.uri().authority(), Some(&Authority::from_static("example.com")));
        assert_eq!(req.headers().get(http::header::HOST).unwrap(), "example.com");

        let head = b"\
                GET / HTTP/1.1\r\n\
                \r\n\
                ";
        let mut buf = BytesMut::from(&head[..]);

        let (req, _) = ctx.decode_head::<128>(&mut buf).unwrap().unwrap();

        assert_eq!(req.uri().scheme(), None);
        assert_eq!(req.uri().authority(), None);
    }
}
