mod error;
mod header;

pub use self::error::MultipartError;

use std::{
    cmp,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Buf, Bytes, BytesMut};
use futures_core::stream::Stream;
use http::{header::HeaderMap, Method, Request};
use memchr::memmem;
use pin_project_lite::pin_project;

/// Multipart protocol using high level API that operate over [Stream] trait.
///
/// `http` crate is used as Http request input. It provides necessary header information needed for
/// [Multipart].
///
/// # Examples:
/// ```rust
/// use std::{convert::Infallible, error};
///
/// use bytes::Bytes;
/// use futures_util::{pin_mut, stream::Stream};
/// use http::Request;
///
/// async fn handle(req: Request<impl Stream<Item = Result<Bytes, Infallible>>>) -> Result<(), Box<dyn error::Error>>{
///     // replace request body type with ()
///     let (parts, body) = req.into_parts();
///     let req = Request::from_parts(parts, ());
///
///     // prepare multipart handling.
///     let mut multipart = http_multipart::multipart(&req, body)?;
///
///     // pin multipart and start await on the request body.
///     pin_mut!(multipart);
///
///     // try individual field of the multipart.
///     while let Some(mut field) = multipart.try_next().await? {
///         // try chunked data for one field.
///         while let Some(chunk) = field.try_next().await? {
///             // handle chunk data.
///         }
///     }
///
///     Ok(())
/// }
/// ```
pub fn multipart<RB, B, T, E>(req: &Request<RB>, body: B) -> Result<Multipart<'_, B>, MultipartError<E>>
where
    B: Stream<Item = Result<T, E>>,
    T: Buf,
{
    multipart_with_config(req, body, Config::default())
}

/// [multipart] with [Config] that used for customize behaviour of [Multipart].
pub fn multipart_with_config<RB, B, T, E>(
    req: &Request<RB>,
    body: B,
    config: Config,
) -> Result<Multipart<'_, B>, MultipartError<E>>
where
    B: Stream<Item = Result<T, E>>,
    T: Buf,
{
    if req.method() != Method::POST {
        return Err(MultipartError::NoPostMethod);
    }

    let boundary = header::boundary(req.headers())?;

    Ok(Multipart {
        stream: body,
        buf: BytesMut::new(),
        boundary,
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
    pub struct Multipart<'a, S> {
        #[pin]
        stream: S,
        buf: BytesMut,
        boundary: &'a [u8],
        config: Config
    }
}

const DOUBLE_HYPHEN: &[u8; 2] = b"--";
const LF: &[u8; 1] = b"\n";
const DOUBLE_CR_LF: &[u8; 4] = b"\r\n\r\n";

impl<'a, S, T, E> Multipart<'a, S>
where
    S: Stream<Item = Result<T, E>>,
    T: Buf,
{
    // take in &mut Pin<&mut Self> so Field can borrow it as Pin<&mut Multipart>.
    // this avoid another explicit stack pin when operating on Field type.
    pub async fn try_next<'s>(self: &'s mut Pin<&mut Self>) -> Result<Option<Field<'s, 'a, S>>, MultipartError<E>> {
        let boundary_len = self.boundary.len();
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

                        let headers = self.as_mut().parse_headers().await?;

                        header::check_headers(&headers)?;

                        let length = header::content_length_opt(&headers)?;

                        return Ok(Some(Field {
                            headers,
                            length,
                            multipart: self.as_mut(),
                        }));
                    }
                    // last boundary.
                    len if len == (boundary_len + 4) => {
                        let at = boundary_len + 2;
                        // TODO: add log for ill formed ending boundary?;
                        let _ = this.boundary.as_ref().eq(&slice[2..at]) && &slice[at..] == DOUBLE_HYPHEN;
                        this.buf.clear();
                        return Ok(None);
                    }
                    // boundary line exceed expected length.
                    _ => return Err(MultipartError::Boundary),
                }
            }

            if self.buf_overflow() {
                return Err(MultipartError::Boundary);
            }

            self.as_mut().try_read_stream_to_buf().await?;
        }
    }

    async fn parse_headers(mut self: Pin<&mut Self>) -> Result<HeaderMap, MultipartError<E>> {
        loop {
            let this = self.as_mut().project();

            if let Some(idx) = memmem::find(this.buf, DOUBLE_CR_LF) {
                let slice = &this.buf[..idx + 4];
                let headers = header::parse_headers(slice)?;
                this.buf.advance(slice.len());

                return Ok(headers);
            }

            if self.buf_overflow() {
                return Err(MultipartError::Header(httparse::Error::TooManyHeaders));
            }

            self.as_mut().try_read_stream_to_buf().await?;
        }
    }

    async fn try_read_stream_to_buf(mut self: Pin<&mut Self>) -> Result<(), MultipartError<E>> {
        let bytes = self.as_mut().try_read_stream().await?;
        self.with_buf(|buf| buf.extend_from_slice(bytes.chunk()));
        Ok(())
    }

    async fn try_read_stream(self: Pin<&mut Self>) -> Result<T, MultipartError<E>> {
        struct Next<'a, S> {
            stream: Pin<&'a mut S>,
        }

        impl<S> Future for Next<'_, S>
        where
            S: Stream,
        {
            type Output = Option<S::Item>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                self.get_mut().stream.as_mut().poll_next(cx)
            }
        }

        let next = Next {
            stream: self.project().stream,
        };

        match next.await {
            Some(Ok(bytes)) => Ok(bytes),
            Some(Err(e)) => Err(MultipartError::Payload(e)),
            None => Err(MultipartError::UnexpectedEof),
        }
    }

    fn with_buf<F, O>(self: Pin<&mut Self>, func: F) -> O
    where
        F: FnOnce(&mut BytesMut) -> O,
    {
        func(self.project().buf)
    }

    fn buf_overflow(&self) -> bool {
        self.buf.len() > self.config.buf_limit
    }
}

pub struct Field<'a, 'b, S> {
    headers: HeaderMap,
    length: Option<u64>,
    multipart: Pin<&'a mut Multipart<'b, S>>,
}

impl<S, T, E> Field<'_, '_, S>
where
    S: Stream<Item = Result<T, E>>,
    T: Buf,
{
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub async fn try_next(&mut self) -> Result<Option<Bytes>, MultipartError<E>> {
        let buf_len = self.multipart.as_mut().with_buf(|buf| buf.len());

        // check multipart buffer first and drain it if possible.
        if buf_len != 0 {
            match self.length.as_mut() {
                Some(0) => return Ok(None),
                Some(len) => {
                    let at = cmp::min(*len, buf_len as u64);
                    *len -= at;

                    let chunk = self
                        .multipart
                        .as_mut()
                        .with_buf(|buf| buf.split_to(at as usize).freeze());

                    return Ok(Some(chunk));
                }
                None => {}
            }
        }

        // multipart buffer is empty. read more from stream.
        let mut bytes = self.multipart.as_mut().try_read_stream().await?;

        // try to deal with the read bytes in place before extend to multipart buffer.
        match self.length.as_mut() {
            Some(0) => {
                self.multipart
                    .as_mut()
                    .with_buf(|buf| buf.extend_from_slice(bytes.chunk()));
                Ok(None)
            }
            Some(len) => {
                let chunk = bytes.chunk();

                let at = cmp::min(*len, chunk.len() as u64);
                *len -= at;

                let at = at as usize;

                let chunk = Bytes::copy_from_slice(&chunk[..at]);

                bytes.advance(at);

                self.multipart
                    .as_mut()
                    .with_buf(|buf| buf.extend_from_slice(bytes.chunk()));

                Ok(Some(chunk))
            }
            None => unimplemented!("multipart field without content length header is not supported yet"),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use futures_util::FutureExt;
    use http::header::{HeaderValue, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};

    fn once_body(b: impl Into<Bytes>) -> impl Stream<Item = Result<Bytes, ()>> {
        futures_util::stream::once(async { Ok::<_, ()>(b.into()) })
    }

    #[test]
    fn method() {
        let req = Request::new(());
        let body = once_body(Bytes::new());
        let err = multipart(&req, body).err();
        assert_eq!(err, Some(MultipartError::NoPostMethod));
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
            Content-Type: text/plain\r\nContent-Length: 8\r\n\r\n\
            testdata\r\n\
            --abbc761f78ff4d7cb7573b5a23f96ef0--\r\n";

        let mut req = Request::new(());
        *req.method_mut() = Method::POST;
        req.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("multipart/mixed; boundary=abbc761f78ff4d7cb7573b5a23f96ef0"),
        );

        let body = once_body(Bytes::copy_from_slice(body));

        let multipart = multipart(&req, body).unwrap();

        futures_util::pin_mut!(multipart);

        let mut field = multipart.try_next().now_or_never().unwrap().unwrap().unwrap();

        assert_eq!(
            field.headers().get(CONTENT_DISPOSITION).unwrap(),
            HeaderValue::from_static("form-data; name=\"file\"; filename=\"foo.txt\"")
        );
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

        let mut field = multipart.try_next().now_or_never().unwrap().unwrap().unwrap();

        assert_eq!(
            field.headers().get(CONTENT_DISPOSITION).unwrap(),
            HeaderValue::from_static("form-data; name=\"file\"; filename=\"bar.txt\"")
        );
        assert_eq!(
            field.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain")
        );
        assert_eq!(
            field.headers().get(CONTENT_LENGTH).unwrap(),
            HeaderValue::from_static("8")
        );
        assert_eq!(
            field.try_next().now_or_never().unwrap().unwrap().unwrap().chunk(),
            b"testdata"
        );
        assert!(field.try_next().now_or_never().unwrap().unwrap().is_none());

        assert!(multipart.try_next().now_or_never().unwrap().unwrap().is_none());
        assert!(multipart.try_next().now_or_never().unwrap().is_err());
    }
}
