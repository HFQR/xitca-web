#![forbid(unsafe_code)]

mod content_disposition;
mod error;
mod field;
mod header;

pub use self::{error::MultipartError, field::Field};

use core::{future::poll_fn, pin::Pin};

use bytes::{Buf, BytesMut};
use field::FieldDecoder;
use futures_core::stream::Stream;
use http::{header::HeaderMap, Method, Request};
use memchr::memmem;
use pin_project_lite::pin_project;

use self::{content_disposition::ContentDisposition, error::PayloadError};

/// Multipart protocol using high level API that operate over [Stream] trait.
///
/// `http` crate is used as Http request input. It provides necessary header information needed for
/// [Multipart].
///
/// # Examples:
/// ```rust
/// use std::{convert::Infallible, error, pin::pin};
///
/// use futures_core::stream::Stream;
/// use http::Request;
///
/// async fn handle<B>(req: Request<B>) -> Result<(), Box<dyn error::Error + Send + Sync>>
/// where
///     B: Stream<Item = Result<Vec<u8>, Infallible>>
/// {
///     // destruct request type.
///     let (parts, body) = req.into_parts();
///     let req = Request::from_parts(parts, ());
///
///     // prepare multipart handling.
///     let mut multipart = http_multipart::multipart(&req, body)?;
///
///     // pin multipart and start await on the request body.
///     let mut multipart = pin!(multipart);
///
///     // try async iterate through fields of the multipart.
///     while let Some(mut field) = multipart.try_next().await? {
///         // try async iterate through single field's bytes data.
///         while let Some(chunk) = field.try_next().await? {
///             // handle bytes data.
///         }
///     }
///
///     Ok(())
/// }
/// ```
pub fn multipart<Ext, B, T, E>(req: &Request<Ext>, body: B) -> Result<Multipart<B>, MultipartError>
where
    B: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
    E: Into<PayloadError>,
{
    multipart_with_config(req, body, Config::default())
}

/// [multipart] with [Config] that used for customize behavior of [Multipart].
pub fn multipart_with_config<Ext, B, T, E>(
    req: &Request<Ext>,
    body: B,
    config: Config,
) -> Result<Multipart<B>, MultipartError>
where
    B: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
    E: Into<PayloadError>,
{
    if req.method() != Method::POST {
        return Err(MultipartError::NoPostMethod);
    }

    let boundary = header::boundary(req.headers())?;

    Ok(Multipart {
        stream: body,
        buf: BytesMut::new(),
        boundary: boundary.into(),
        headers: HeaderMap::new(),
        pending_field: false,
        config,
    })
}

/// Configuration for [Multipart] type
#[derive(Debug, Copy, Clone)]
pub struct Config {
    /// limit the max size of internal buffer.
    /// internal buffer is used to cache overlapped chunks around boundary and filed headers.
    /// Default to 1MB
    pub buf_limit: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { buf_limit: 1024 * 1024 }
    }
}

pin_project! {
    pub struct Multipart<S> {
        #[pin]
        stream: S,
        buf: BytesMut,
        boundary: Box<[u8]>,
        headers: HeaderMap,
        pending_field: bool,
        config: Config
    }
}

const DOUBLE_HYPHEN: &[u8; 2] = b"--";
const LF: &[u8; 1] = b"\n";
const DOUBLE_CR_LF: &[u8; 4] = b"\r\n\r\n";

impl<S, T, E> Multipart<S>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
    E: Into<PayloadError>,
{
    // take in &mut Pin<&mut Self> so Field can borrow it as Pin<&mut Multipart>.
    // this avoid another explicit stack pin when operating on Field type.
    pub async fn try_next<'s>(self: &'s mut Pin<&mut Self>) -> Result<Option<Field<'s, S>>, MultipartError> {
        let boundary_len = self.boundary.len();

        if self.pending_field {
            self.as_mut().consume_pending_field().await?;
        }

        loop {
            let this = self.as_mut().project();
            if let Some(idx) = memmem::find(this.buf, LF) {
                // backtrack one byte to exclude CR
                let slice = match idx.checked_sub(1) {
                    Some(idx) => &this.buf[..idx],
                    // no CR before LF.
                    None => return Err(MultipartError::Boundary),
                };

                match slice.len() {
                    // empty line. skip.
                    0 => {
                        // forward one byte to include LF and remove the empty line.
                        this.buf.advance(idx + 1);
                        continue;
                    }
                    // not enough data to operate.
                    len if len < (boundary_len + 2) => {}
                    // not boundary.
                    _ if &slice[..2] != DOUBLE_HYPHEN => return Err(MultipartError::Boundary),
                    // non last boundary
                    _ if this.boundary.as_ref().eq(&slice[2..]) => {
                        // forward one byte to include CRLF and remove the boundary line.
                        this.buf.advance(idx + 1);

                        let field = self.as_mut().parse_field().await?;
                        return Ok(Some(field));
                    }
                    // last boundary.
                    len if len == (boundary_len + 4) => {
                        let at = boundary_len + 2;
                        // TODO: add log for ill formed ending boundary?;
                        let _ = this.boundary.as_ref().eq(&slice[2..at]) && &slice[at..] == DOUBLE_HYPHEN;
                        return Ok(None);
                    }
                    // boundary line exceed expected length.
                    _ => return Err(MultipartError::Boundary),
                }
            }

            if self.buf_overflow() {
                return Err(MultipartError::BufferOverflow);
            }

            self.as_mut().try_read_stream_to_buf().await?;
        }
    }

    async fn parse_field(mut self: Pin<&mut Self>) -> Result<Field<'_, S>, MultipartError> {
        loop {
            let this = self.as_mut().project();

            if let Some(idx) = memmem::find(this.buf, DOUBLE_CR_LF) {
                let slice = &this.buf[..idx + 4];

                header::parse_headers(this.headers, slice)?;
                this.buf.advance(slice.len());

                let cp = ContentDisposition::try_from_header(this.headers)?;

                header::check_headers(this.headers)?;

                let length = header::content_length_opt(this.headers)?;

                *this.pending_field = true;

                return Ok(Field::new(length, cp, self));
            }

            if self.buf_overflow() {
                return Err(MultipartError::Header(httparse::Error::TooManyHeaders));
            }

            self.as_mut().try_read_stream_to_buf().await?;
        }
    }

    #[cold]
    #[inline(never)]
    async fn consume_pending_field(mut self: Pin<&mut Self>) -> Result<(), MultipartError> {
        let mut field_ty = FieldDecoder::default();

        loop {
            let this = self.as_mut().project();
            if let Some(idx) = field_ty.try_find_split_idx(this.buf, this.boundary)? {
                this.buf.advance(idx);
            }
            if matches!(field_ty, FieldDecoder::StreamEnd) {
                *this.pending_field = false;
                return Ok(());
            }
            self.as_mut().try_read_stream_to_buf().await?;
        }
    }

    async fn try_read_stream_to_buf(mut self: Pin<&mut Self>) -> Result<(), MultipartError> {
        let bytes = self.as_mut().try_read_stream().await?;
        self.project().buf.extend_from_slice(bytes.as_ref());
        Ok(())
    }

    async fn try_read_stream(mut self: Pin<&mut Self>) -> Result<T, MultipartError> {
        match poll_fn(move |cx| self.as_mut().project().stream.poll_next(cx)).await {
            Some(Ok(bytes)) => Ok(bytes),
            Some(Err(e)) => Err(MultipartError::Payload(e.into())),
            None => Err(MultipartError::UnexpectedEof),
        }
    }

    pub(crate) fn buf_overflow(&self) -> bool {
        self.buf.len() > self.config.buf_limit
    }
}

#[cfg(test)]
mod test {
    use std::{convert::Infallible, pin::pin};

    use bytes::Bytes;
    use futures_util::FutureExt;
    use http::header::{HeaderValue, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};

    use super::*;

    fn once_body(b: impl Into<Bytes>) -> impl Stream<Item = Result<Bytes, Infallible>> {
        futures_util::stream::once(async { Ok(b.into()) })
    }

    #[test]
    fn method() {
        let req = Request::new(());
        let body = once_body(Bytes::new());
        let err = multipart(&req, body).err();
        assert!(matches!(err, Some(MultipartError::NoPostMethod)));
    }

    #[test]
    fn basic() {
        let body = b"\
            --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
            Content-Disposition: form-data; name=\"file\"; filename=\"foo.txt\"\r\n\
            Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
            test\r\n\
            --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
            Content-Disposition: form-data; name=\"file\"; filename=\"bar.txt\"\r\n\
            Content-Type: text/plain\r\n\r\n\
            testdata\r\n\
            --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
            Content-Disposition: form-data; name=\"file\"; filename=\"bar.txt\"\r\n\
            Content-Type: text/plain\r\n\r\n\
            testdata\r\n\
            --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
            Content-Disposition: form-data; name=\"file\"; filename=\"bar.txt\"\r\n\
            Content-Type: text/plain\r\nContent-Length: 9\r\n\r\n\
            testdata2\r\n\
            --abbc761f78ff4d7cb7573b5a23f96ef0--\r\n\
            ";

        let mut req = Request::new(());
        *req.method_mut() = Method::POST;
        req.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("multipart/mixed; boundary=abbc761f78ff4d7cb7573b5a23f96ef0"),
        );

        let body = once_body(Bytes::copy_from_slice(body));

        let multipart = multipart(&req, body).unwrap();

        let mut multipart = pin!(multipart);

        {
            let mut field = multipart.try_next().now_or_never().unwrap().unwrap().unwrap();

            assert_eq!(
                field.headers().get(CONTENT_DISPOSITION).unwrap(),
                HeaderValue::from_static("form-data; name=\"file\"; filename=\"foo.txt\"")
            );
            assert_eq!(field.name().unwrap(), "file");
            assert_eq!(field.file_name().unwrap(), "foo.txt");
            assert_eq!(
                field.headers().get(CONTENT_TYPE).unwrap(),
                HeaderValue::from_static("text/plain; charset=utf-8")
            );
            assert_eq!(
                field.headers().get(CONTENT_LENGTH).unwrap(),
                HeaderValue::from_static("4")
            );
            assert_eq!(
                field.try_next().now_or_never().unwrap().unwrap().unwrap().chunk(),
                b"test"
            );
            assert!(field.try_next().now_or_never().unwrap().unwrap().is_none());
        }

        {
            let mut field = multipart.try_next().now_or_never().unwrap().unwrap().unwrap();

            assert_eq!(
                field.headers().get(CONTENT_DISPOSITION).unwrap(),
                HeaderValue::from_static("form-data; name=\"file\"; filename=\"bar.txt\"")
            );
            assert_eq!(field.name().unwrap(), "file");
            assert_eq!(field.file_name().unwrap(), "bar.txt");
            assert_eq!(
                field.headers().get(CONTENT_TYPE).unwrap(),
                HeaderValue::from_static("text/plain")
            );
            assert!(field.headers().get(CONTENT_LENGTH).is_none());
            assert_eq!(
                field.try_next().now_or_never().unwrap().unwrap().unwrap().chunk(),
                b"testdata"
            );
            assert!(field.try_next().now_or_never().unwrap().unwrap().is_none());
        }

        // test drop field without consuming.
        multipart.try_next().now_or_never().unwrap().unwrap().unwrap();

        {
            let mut field = multipart.try_next().now_or_never().unwrap().unwrap().unwrap();

            assert_eq!(
                field.headers().get(CONTENT_DISPOSITION).unwrap(),
                HeaderValue::from_static("form-data; name=\"file\"; filename=\"bar.txt\"")
            );
            assert_eq!(field.name().unwrap(), "file");
            assert_eq!(field.file_name().unwrap(), "bar.txt");
            assert_eq!(
                field.headers().get(CONTENT_TYPE).unwrap(),
                HeaderValue::from_static("text/plain")
            );
            assert_eq!(
                field.headers().get(CONTENT_LENGTH).unwrap(),
                HeaderValue::from_static("9")
            );
            assert_eq!(
                field.try_next().now_or_never().unwrap().unwrap().unwrap().chunk(),
                b"testdata2"
            );
            assert!(field.try_next().now_or_never().unwrap().unwrap().is_none());
        }

        assert!(multipart.try_next().now_or_never().unwrap().unwrap().is_none());
        assert!(multipart.try_next().now_or_never().unwrap().unwrap().is_none());
    }

    #[test]
    fn field_header_overflow() {
        let body = b"\
            --12345\r\n\
            Content-Disposition: form-data; name=\"file\"; filename=\"foo.txt\"\r\n\
            Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4";

        let mut req = Request::new(());
        *req.method_mut() = Method::POST;
        req.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("multipart/mixed; boundary=12345"),
        );

        let body = once_body(Bytes::copy_from_slice(body));

        // limit is set to 7 so the first boundary can be parsed.
        let multipart = multipart_with_config(&req, body, Config { buf_limit: 7 }).unwrap();

        let mut multipart = pin!(multipart);

        assert!(matches!(
            multipart.try_next().now_or_never().unwrap().err().unwrap(),
            MultipartError::Header(httparse::Error::TooManyHeaders)
        ));
    }

    #[test]
    fn boundary_overflow() {
        let body = b"--123456";

        let mut req = Request::new(());
        *req.method_mut() = Method::POST;
        req.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("multipart/mixed; boundary=12345"),
        );

        let body = once_body(Bytes::copy_from_slice(body));

        // limit is set to 7 so the first boundary can not be parsed.
        let multipart = multipart_with_config(&req, body, Config { buf_limit: 7 }).unwrap();

        let mut multipart = pin!(multipart);

        assert!(matches!(
            multipart.try_next().now_or_never().unwrap().err().unwrap(),
            MultipartError::BufferOverflow
        ));
    }
}
