use bytes::{BufMut, BytesMut};
use http::{response::Parts, StatusCode, Version};
use log::{debug, warn};

use crate::body::ResponseBodySize;

use super::context::Context;
use super::error::{Parse, ProtoError};

impl Context {
    pub(super) fn encode_head(
        &mut self,
        mut parts: Parts,
        size: ResponseBodySize,
        buf: &mut BytesMut,
    ) -> Result<(), ProtoError> {
        let version = parts.version;
        let status = parts.status;

        let skip_len = match (status, version) {
            (StatusCode::SWITCHING_PROTOCOLS, _) => true,
            // TODO: add Request method to context and check in following branch.
            // Connect method with success status code should have no body.
            (s, _) if s.is_success() => true,
            (s, _) if s.is_informational() => {
                warn!("response with 1xx status code not supported");
                return Err(ProtoError::Parse(Parse::StatusCode));
            }
            _ => false,
        };

        // In some error cases, we don't know about the invalid message until already
        // pushing some bytes onto the `buf`. In those cases, we don't want to send
        // the half-pushed message, so rewind to before.
        let orig_len = buf.len();

        // encode version, status code and reason
        encode_version_status_reason(buf, version, status);

        for (name, value) in parts.headers.drain() {
            let name = name.expect("Handling optional header name is not implemented");

            buf.put_slice(name.as_str().as_bytes());
            buf.put_slice(b": ");
            buf.put_slice(value.as_bytes());
            buf.put_slice(b"\r\n");
        }

        buf.put_slice(b"\r\nHello World!\r\n");

        // put header map back to cache.
        self.header_cache = Some(parts.headers);

        Ok(())
    }
}

fn encode_version_status_reason<B: BufMut>(buf: &mut B, version: Version, status: StatusCode) {
    // encode version, status code and reason
    match (version, status) {
        // happy path shortcut.
        (Version::HTTP_11, StatusCode::OK) => {
            buf.put_slice(b"HTTP/1.1 200 OK\r\n");
            return;
        }
        (Version::HTTP_10, _) => {
            buf.put_slice(b"HTTP/1.0 ");
        }
        (Version::HTTP_11, _) => {
            buf.put_slice(b"HTTP/1.1 ");
        }
        _ => {
            debug!("response with unexpected response version");
            buf.put_slice(b"HTTP/1.1 ");
        }
    }

    buf.put_slice(status.as_str().as_bytes());
    buf.put_slice(b" ");
    // a reason MUST be written, as many parsers will expect it.
    buf.put_slice(status.canonical_reason().unwrap_or("<none>").as_bytes());
    buf.put_slice(b"\r\n");
}
