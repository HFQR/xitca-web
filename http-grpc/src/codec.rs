use bytes::{BufMut, BytesMut};
use http::HeaderMap;
use prost::Message;

#[cfg(feature = "__compress")]
use http_body_alt::{Body, Frame, util::Full};
#[cfg(feature = "__compress")]
use http_encoding::ContentEncoding;

use super::error::ProtocolError;

/// Default body size limit for gRPC messages (4 MiB).
pub const DEFAULT_LIMIT: usize = 4 * 1024 * 1024;

#[cfg(feature = "__compress")]
const GRPC_ENCODING: http::HeaderName = http::HeaderName::from_static("grpc-encoding");

/// gRPC length-prefixed framing codec.
///
/// Handles the 5-byte gRPC frame header (1 byte compression flag + 4 byte big-endian length)
/// and protobuf encode/decode with optional compression.
pub struct Codec {
    limit: usize,
    #[cfg(feature = "__compress")]
    encoding: ContentEncoding,
}

impl Codec {
    pub fn new() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            #[cfg(feature = "__compress")]
            encoding: Default::default(),
        }
    }

    #[allow(unused_variables)]
    pub fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            #[cfg(feature = "__compress")]
            encoding: ContentEncoding::from_headers_with(headers, &GRPC_ENCODING),
        }
    }

    /// Set the maximum allowed size in bytes for a single gRPC message frame.
    /// Set to `0` for unlimited.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }

    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Set the content encoding for compression/decompression.
    #[cfg(feature = "__compress")]
    pub fn set_encoding(mut self, encoding: ContentEncoding) -> Self {
        self.encoding = encoding;
        self
    }

    /// Try to decode a complete gRPC message from `src`.
    ///
    /// Consumes the frame bytes from `src` on success.
    ///
    /// Returns:
    /// - `Ok(Some(message))` when a complete frame is available
    /// - `Ok(None)` when more data is needed
    /// - `Err` on protocol violations (size limit, decode error)
    pub fn decode<T: Message + Default>(&self, src: &mut BytesMut) -> Result<Option<T>, ProtocolError> {
        if src.len() < 5 {
            return Ok(None);
        }

        let compressed = src[0] != 0;
        let len = u32::from_be_bytes(src[1..5].try_into().unwrap()) as usize;

        if self.limit > 0 && len > self.limit {
            return Err(ProtocolError::MessageTooLarge {
                size: len,
                limit: self.limit,
            });
        }

        if src.len() < 5 + len {
            return Ok(None);
        }

        let _ = src.split_to(5);
        let payload = src.split_to(len);

        let payload = if compressed { self.decompress(payload)? } else { payload };

        let msg = Message::decode(payload).map_err(ProtocolError::Decode)?;

        Ok(Some(msg))
    }

    /// Encode a protobuf message into gRPC length-prefixed framing.
    ///
    /// Writes to `dst`: 1 byte compression flag + 4 byte big-endian length + payload.
    /// When compression is enabled and the `compress` feature is active, the payload
    /// is compressed and the flag byte is set to 1.
    pub fn encode<T: Message>(&self, msg: &T, dst: &mut BytesMut) -> Result<(), ProtocolError> {
        let encoded_len = msg.encoded_len();
        dst.reserve(5 + encoded_len);
        dst.put_u8(0); // compression flag placeholder
        dst.put_u32(0); // length placeholder
        msg.encode(dst).map_err(ProtocolError::Encode)?;

        self.compress(dst)?;

        // write actual payload length
        let len = (dst.len() - 5) as u32;
        dst[1..5].copy_from_slice(&len.to_be_bytes());

        Ok(())
    }

    #[cfg(feature = "__compress")]
    fn decompress(&self, payload: BytesMut) -> Result<BytesMut, ProtocolError> {
        if matches!(self.encoding, ContentEncoding::Identity) {
            return Err(ProtocolError::CompressedWithoutEncoding);
        }

        let body = self.encoding.decode_body(Full::new(payload));
        let mut body = core::pin::pin!(body);
        let mut out = BytesMut::new();

        // drive synchronously — Full body yields once and never returns Pending
        let waker = core::task::Waker::noop();
        let mut cx = core::task::Context::from_waker(waker);
        loop {
            match Body::poll_frame(body.as_mut(), &mut cx) {
                core::task::Poll::Ready(Some(Ok(Frame::Data(data)))) => {
                    out.extend_from_slice(data.as_ref());
                }
                core::task::Poll::Ready(Some(Err(e))) => {
                    return Err(ProtocolError::Compress(e.to_string()));
                }
                core::task::Poll::Ready(None | Some(Ok(Frame::Trailers(_)))) => break,
                core::task::Poll::Pending => unreachable!("Full body never returns Pending"),
            }
        }

        Ok(out)
    }

    #[cfg(not(feature = "__compress"))]
    fn decompress(&self, _: BytesMut) -> Result<BytesMut, ProtocolError> {
        Err(ProtocolError::CompressUnsupported)
    }

    #[cfg(feature = "__compress")]
    fn compress(&self, dst: &mut BytesMut) -> Result<(), ProtocolError> {
        if matches!(self.encoding, ContentEncoding::Identity) {
            return Ok(());
        }

        let payload = dst.split_off(5);
        let body = self.encoding.encode_body(Full::new(payload));
        let mut body = core::pin::pin!(body);

        // clear and rewrite header
        dst.clear();
        dst.put_u8(1); // compressed flag
        dst.put_u32(0); // length placeholder

        let waker = core::task::Waker::noop();
        let mut cx = core::task::Context::from_waker(waker);
        loop {
            match Body::poll_frame(body.as_mut(), &mut cx) {
                core::task::Poll::Ready(Some(Ok(Frame::Data(data)))) => {
                    dst.extend_from_slice(data.as_ref());
                }
                core::task::Poll::Ready(Some(Err(e))) => {
                    return Err(ProtocolError::Compress(e.to_string()));
                }
                core::task::Poll::Ready(None | Some(Ok(Frame::Trailers(_)))) => break,
                core::task::Poll::Pending => unreachable!("Full body never returns Pending"),
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "__compress"))]
    fn compress(&self, _: &mut BytesMut) -> Result<(), ProtocolError> {
        Ok(())
    }
}
