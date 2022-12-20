use futures_core::stream::Stream;
use tracing::{debug, warn};

use crate::{
    body::BodySize,
    bytes::BytesMut,
    date::DateTime,
    http::{
        header::{HeaderMap, CONNECTION, CONTENT_LENGTH, DATE, TE, TRANSFER_ENCODING},
        response::Parts,
        Extensions, StatusCode, Version,
    },
};

use super::{
    buf_write::H1BufWrite,
    codec::TransferCoding,
    context::{ConnectionType, Context},
    error::{Parse, ProtoError},
};

impl<D, const MAX_HEADERS: usize> Context<'_, D, MAX_HEADERS>
where
    D: DateTime,
{
    pub(super) fn encode_continue<W>(&mut self, buf: &mut W)
    where
        W: H1BufWrite,
    {
        buf.write_static(b"HTTP/1.1 100 Continue\r\n\r\n");
    }

    pub fn encode_head<B, W>(&mut self, parts: Parts, body: &B, buf: &mut W) -> Result<TransferCoding, ProtoError>
    where
        B: Stream,
        W: H1BufWrite,
    {
        buf.write_head(|buf| self.encode_head_inner(parts, body, buf))
    }

    fn encode_head_inner<B>(&mut self, parts: Parts, body: &B, buf: &mut BytesMut) -> Result<TransferCoding, ProtoError>
    where
        B: Stream,
    {
        let version = parts.version;
        let status = parts.status;

        // decide if content-length or transfer-encoding header would be skipped.
        let skip_len = match (status, version) {
            (StatusCode::SWITCHING_PROTOCOLS, _) => true,
            // Sending content-length or transfer-encoding header on 2xx response
            // to CONNECT is forbidden in RFC 7231.
            (s, _) if self.is_connect_method() && s.is_success() => true,
            (s, _) if s.is_informational() => {
                warn!(target: "h1_encode", "response with 1xx status code not supported");
                return Err(ProtoError::Parse(Parse::StatusCode));
            }
            _ => false,
        };

        // In some error cases, we don't know about the invalid message until already
        // pushing some bytes onto the `buf`. In those cases, we don't want to send
        // the half-pushed message, so rewind to before.
        // let orig_len = buf.len();

        // encode version, status code and reason
        encode_version_status_reason(buf, version, status);

        self.encode_headers(parts.headers, parts.extensions, body, buf, skip_len)
    }
}

#[inline]
fn encode_version_status_reason(buf: &mut BytesMut, version: Version, status: StatusCode) {
    // encode version, status code and reason
    match (version, status) {
        // happy path shortcut.
        (Version::HTTP_11, StatusCode::OK) => {
            buf.extend_from_slice(b"HTTP/1.1 200 OK");
            return;
        }
        (Version::HTTP_11, _) => {
            buf.extend_from_slice(b"HTTP/1.1 ");
        }
        (Version::HTTP_10, _) => {
            buf.extend_from_slice(b"HTTP/1.0 ");
        }
        _ => {
            debug!(target: "h1_encode", "response with unexpected response version");
            buf.extend_from_slice(b"HTTP/1.1 ");
        }
    }

    // a reason MUST be written, as many parsers will expect it.
    let reason = status.canonical_reason().unwrap_or("<none>").as_bytes();
    let status = status.as_str().as_bytes();

    buf.reserve(status.len() + reason.len() + 1);
    buf.extend_from_slice(status);
    buf.extend_from_slice(b" ");
    buf.extend_from_slice(reason);
}

impl<D, const MAX_HEADERS: usize> Context<'_, D, MAX_HEADERS>
where
    D: DateTime,
{
    pub fn encode_headers<B>(
        &mut self,
        mut headers: HeaderMap,
        mut extensions: Extensions,
        body: &B,
        buf: &mut BytesMut,
        mut skip_len: bool,
    ) -> Result<TransferCoding, ProtoError>
    where
        B: Stream,
    {
        let size = BodySize::from_stream(body);

        let mut skip_date = false;

        let mut encoding = TransferCoding::eof();

        // use the shortest header name as default
        let mut name = TE;

        for (next_name, value) in headers.drain() {
            let is_continue = match next_name {
                Some(next_name) => {
                    name = next_name;
                    false
                }
                None => true,
            };

            match name {
                CONTENT_LENGTH => {
                    debug_assert!(!skip_len, "CONTENT_LENGTH header can not be set");
                    let value = value
                        .to_str()
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .ok_or(Parse::HeaderValue)?;
                    encoding = TransferCoding::length(value);
                    skip_len = true;
                }
                TRANSFER_ENCODING => {
                    debug_assert!(!skip_len, "TRANSFER_ENCODING header can not be set");
                    encoding = TransferCoding::encode_chunked();
                    skip_len = true;
                }
                CONNECTION => match self.ctype() {
                    // skip write header on close condition.
                    // the header is checked again and written properly afterwards.
                    ConnectionType::Close => continue,
                    _ => {
                        for val in value.to_str().map_err(|_| Parse::HeaderValue)?.split(',') {
                            let val = val.trim();
                            if val.eq_ignore_ascii_case("close") {
                                self.set_ctype(ConnectionType::Close);
                            } else if val.eq_ignore_ascii_case("keep-alive") {
                                self.set_ctype(ConnectionType::KeepAlive);
                            } else if val.eq_ignore_ascii_case("upgrade") {
                                encoding = TransferCoding::upgrade();
                                break;
                            }
                        }
                    }
                },
                DATE => skip_date = true,
                _ => {}
            }

            let value = value.as_bytes();

            if is_continue {
                buf.reserve(value.len() + 1);
                buf.extend_from_slice(b",");
                buf.extend_from_slice(value);
            } else {
                let name = name.as_str().as_bytes();

                buf.reserve(name.len() + value.len() + 4);
                buf.extend_from_slice(b"\r\n");
                buf.extend_from_slice(name);
                buf.extend_from_slice(b": ");
                buf.extend_from_slice(value);
            }
        }

        if matches!(self.ctype(), ConnectionType::Close) {
            buf.extend_from_slice(b"\r\nconnection: close");
        }

        // encode transfer-encoding or content-length
        if !skip_len {
            match size {
                BodySize::None => {
                    encoding = TransferCoding::eof();
                }
                BodySize::Stream => {
                    buf.extend_from_slice(b"\r\ntransfer-encoding: chunked");
                    encoding = TransferCoding::encode_chunked();
                }
                BodySize::Sized(size) => {
                    let mut buffer = itoa::Buffer::new();
                    let buffer = buffer.format(size).as_bytes();

                    buf.reserve(buffer.len() + 18);
                    buf.extend_from_slice(b"\r\ncontent-length: ");
                    buf.extend_from_slice(buffer);

                    encoding = TransferCoding::length(size as u64);
                }
            }
        }

        if self.is_head_method() {
            debug_assert_eq!(
                size,
                BodySize::None,
                "Response to request with HEAD method must not have a body"
            );
            encoding = TransferCoding::eof();
        }

        // set date header if there is not any.
        if !skip_date {
            buf.reserve(D::DATE_VALUE_LENGTH + 12);
            buf.extend_from_slice(b"\r\ndate: ");
            self.date.with_date(|slice| buf.extend_from_slice(slice));
        }

        buf.extend_from_slice(b"\r\n\r\n");

        // put header map back to cache.
        self.replace_headers(headers);

        // put extension back to cache;
        extensions.clear();
        self.replace_extensions(extensions);

        Ok(encoding)
    }
}

#[cfg(test)]
mod test {
    use crate::{
        body::{Once, StreamBody},
        bytes::Bytes,
        date::DateTimeService,
        http::{HeaderValue, Response},
    };

    use super::*;

    #[tokio::test]
    async fn append_header() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let date = DateTimeService::new();
                let mut ctx = Context::<_, 64>::new(date.get());

                let mut res = Response::new(StreamBody::new(Once::new(Bytes::new())));

                res.headers_mut()
                    .insert(CONNECTION, HeaderValue::from_static("keep-alive"));
                res.headers_mut()
                    .append(CONNECTION, HeaderValue::from_static("upgrade"));

                let (parts, body) = res.into_parts();

                let mut buf = BytesMut::new();
                ctx.encode_head(parts, &body, &mut buf).unwrap();

                let mut header = [httparse::EMPTY_HEADER; 8];
                let mut res = httparse::Response::new(&mut header);

                let httparse::Status::Complete(_) = res.parse(buf.as_ref()).unwrap()
                    else { panic!("failed to parse response") };

                for h in header {
                    if h.name == "connection" {
                        assert_eq!(h.value, b"keep-alive,upgrade");
                    }
                }
            })
            .await
    }
}
