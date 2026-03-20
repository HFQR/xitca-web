use core::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::{borrow::Cow, io, vec};

use bytes::{Bytes, BytesMut};
use futures_core::stream::Stream;
use http::header::HeaderValue;

use super::MultipartError;

type BoxedStream = Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send>>;

/// A builder for a `multipart/form-data` request body.
///
/// Add fields via [`Part`], then obtain the `Content-Type` header value with
/// [`Form::content_type`] and the body stream with [`Form::into_stream`].
///
/// # Examples
/// ```rust
/// use http_multipart::{Form, Part};
///
/// let parts = vec![
///     Part::text("username", "alice"),
///     Part::binary("data", &b"hello"[..]).file_name("hello.bin"),
/// ];
/// let form = Form::new(parts);
///
/// let content_type = form.content_type(); // set on the outgoing Request
/// // `form` itself implements Stream — use it directly as the request body
/// ```
pub struct Form {
    boundary: Box<[u8]>,
    parts: vec::IntoIter<Part>,
    state: StreamState,
    buf: BytesMut,
}

/// A single field in a multipart form body.
pub struct Part {
    name: String,
    filename: Option<String>,
    content_type: Cow<'static, str>,
    body: BoxedStream,
}

impl Form {
    /// Create a new form with an automatically generated boundary.
    pub fn new(parts: Vec<Part>) -> Self {
        Self::with_boundary(generate_boundary(), parts).unwrap()
    }

    /// Create a form with a caller-supplied boundary.
    ///
    /// Returns [`MultipartError::Boundary`] if `boundary` is empty or contains
    /// `\r` / `\n` (which would break the wire format).
    pub fn with_boundary(boundary: impl AsRef<[u8]>, parts: Vec<Part>) -> Result<Self, MultipartError> {
        let b = boundary.as_ref();
        if b.is_empty() || b.iter().any(|&c| c == b'\r' || c == b'\n') {
            return Err(MultipartError::Boundary);
        }
        let mut prefixed = Vec::with_capacity(2 + b.len());
        prefixed.extend_from_slice(b"--");
        prefixed.extend_from_slice(b);
        Ok(Self {
            boundary: prefixed.into_boxed_slice(),
            parts: parts.into_iter(),
            state: StreamState::NextPart,
            buf: BytesMut::new(),
        })
    }

    /// The raw boundary bytes used by this form.
    pub fn boundary(&self) -> &[u8] {
        &self.boundary[2..]
    }

    /// The `Content-Type` header value that must be set on the outgoing request.
    ///
    /// Example: `multipart/form-data; boundary=0000000000001a2b`
    pub fn content_type(&self) -> HeaderValue {
        let boundary = self.boundary();
        let mut v = BytesMut::with_capacity(30 + boundary.len());
        v.extend_from_slice(b"multipart/form-data; boundary=");
        v.extend_from_slice(boundary);
        // Boundary is validated to be ASCII on construction, so this never panics.
        HeaderValue::from_maybe_shared(v.freeze()).expect("boundary is valid ASCII")
    }
}

impl Part {
    /// plain-text field. field `Content-Type` is `text/plain; charset=utf-8`.
    #[inline]
    pub fn text(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self::binary(name, value.into()).content_type("text/plain; charset=utf-8")
    }

    /// fixed binary field. field `Content-Type` is `application/octet-stream`.
    #[inline]
    pub fn binary(name: impl Into<String>, body: impl Into<Bytes>) -> Self {
        Self::stream(name, Once(Some(body.into())))
    }

    /// streaming binary field. field `Content-Type` is `application/octet-stream`.
    ///
    /// [`Stream`] trait is utilized for generic async streaming interface
    pub fn stream<S>(name: impl Into<String>, stream: S) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        Self {
            name: name.into(),
            filename: None,
            content_type: Cow::Borrowed("application/octet-stream"),
            body: Box::pin(stream),
        }
    }

    /// Attach a filename (sets the `filename` parameter in `Content-Disposition`).
    pub fn file_name(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Override the `Content-Type` for this part.
    pub fn content_type(mut self, ct: impl Into<Cow<'static, str>>) -> Self {
        self.content_type = ct.into();
        self
    }

    fn format_headers_into(&self, buf: &mut BytesMut) {
        let (low, up) = self.body.size_hint();
        let exact = Some(low) == up;

        // Fixed bytes:
        //   "Content-Disposition: form-data; name=\""  38
        //   closing quote + CRLF                        3
        //   "Content-Type: " + CRLF                    16
        //   final blank CRLF                             2
        //                                         total 59
        let mut len = 59 + self.name.len() + self.content_type.len();
        if let Some(fname) = &self.filename {
            len += 13 + fname.len(); // "; filename=\"" (12) + closing quote (1)
        }
        if exact {
            len += 38; // "Content-Length: " (16) + up to 20 digits + CRLF (2)
        }
        buf.reserve(len);

        buf.extend_from_slice(b"Content-Disposition: form-data; name=\"");
        buf.extend_from_slice(self.name.as_bytes());
        buf.extend_from_slice(b"\"");

        if let Some(fname) = &self.filename {
            buf.extend_from_slice(b"; filename=\"");
            buf.extend_from_slice(fname.as_bytes());
            buf.extend_from_slice(b"\"");
        }

        buf.extend_from_slice(b"\r\n");

        buf.extend_from_slice(b"Content-Type: ");
        buf.extend_from_slice(self.content_type.as_bytes());
        buf.extend_from_slice(b"\r\n");

        if exact {
            buf.extend_from_slice(b"Content-Length: ");
            buf.extend_from_slice(format!("{low}").as_bytes());
            buf.extend_from_slice(b"\r\n");
        }

        buf.extend_from_slice(b"\r\n");
    }
}

struct Once(Option<Bytes>);

impl Stream for Once {
    type Item = io::Result<Bytes>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().0.take().map(Ok))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.0.as_ref().map(|b| b.len()).unwrap_or(0);
        (size, Some(size))
    }
}

enum StreamState {
    NextPart,
    Body(BoxedStream),
    Done,
}

impl Stream for Form {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match &mut this.state {
            StreamState::Done => Poll::Ready(None),
            StreamState::NextPart => {
                this.buf.reserve(this.boundary.len() + 4);
                this.buf.extend_from_slice(&this.boundary);

                match this.parts.next() {
                    None => {
                        this.buf.extend_from_slice(b"--\r\n");
                        this.state = StreamState::Done;
                    }
                    Some(part) => {
                        this.buf.extend_from_slice(b"\r\n");
                        part.format_headers_into(&mut this.buf);
                        this.state = StreamState::Body(part.body);
                    }
                };

                Poll::Ready(Some(Ok(this.buf.split().freeze())))
            }
            StreamState::Body(body) => {
                let chunk = ready!(body.as_mut().poll_next(cx)).unwrap_or_else(|| {
                    this.state = StreamState::NextPart;
                    Ok(Bytes::from_static(b"\r\n"))
                });

                Poll::Ready(Some(chunk))
            }
        }
    }
}

// Generate a boundary that is unique within the current process.
//
// The boundary is not cryptographically random.  For environments where the
// boundary must not appear in the body (security-sensitive uploads), supply
// your own random boundary via [`Form::with_boundary`].
fn generate_boundary() -> Box<[u8]> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // XOR with a stack address for modest per-process entropy.
    let salt = &n as *const u64 as u64;
    let val = n ^ salt.wrapping_mul(0x9e3779b97f4a7c15);
    format!("{val:016x}").into_bytes().into_boxed_slice()
}

#[cfg(test)]
mod test {
    use std::{convert::Infallible, pin::pin};

    use bytes::Bytes;
    use futures_util::{FutureExt, StreamExt};
    use http::{header::CONTENT_TYPE, Method, Request};

    use super::*;
    use crate::multipart;

    /// Drain a `Form` synchronously. Works because `Once` bodies are always `Poll::Ready`.
    fn collect(mut form: Form) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            match form.next().now_or_never() {
                Some(Some(Ok(bytes))) => out.extend_from_slice(&bytes),
                Some(None) => break,
                Some(Some(Err(e))) => panic!("stream error: {e}"),
                None => panic!("stream returned Poll::Pending unexpectedly"),
            }
        }
        out
    }

    #[test]
    fn empty_form() {
        let form = Form::with_boundary("abc", vec![]).unwrap();
        let body = collect(form);
        assert_eq!(body, b"--abc--\r\n");
    }

    #[test]
    fn single_text_field() {
        // Part::text uses Once whose size_hint is exact, so Content-Length is emitted.
        let form = Form::with_boundary("abc", vec![Part::text("field", "value")]).unwrap();
        let body = collect(form);
        assert_eq!(
            body,
            b"--abc\r\n\
              Content-Disposition: form-data; name=\"field\"\r\n\
              Content-Type: text/plain; charset=utf-8\r\n\
              Content-Length: 5\r\n\
              \r\n\
              value\r\n\
              --abc--\r\n"
        );
    }

    #[test]
    fn file_part() {
        let part = Part::binary("upload", Bytes::from_static(b"data"))
            .file_name("hello.bin")
            .content_type("application/octet-stream");
        let form = Form::with_boundary("abc", vec![part]).unwrap();
        let body = collect(form);
        assert_eq!(
            body,
            b"--abc\r\n\
              Content-Disposition: form-data; name=\"upload\"; filename=\"hello.bin\"\r\n\
              Content-Type: application/octet-stream\r\n\
              Content-Length: 4\r\n\
              \r\n\
              data\r\n\
              --abc--\r\n"
        );
    }

    #[test]
    fn roundtrip() {
        // Encode with Form, then parse with Multipart and verify fields.
        let parts = vec![
            Part::text("name", "alice"),
            Part::binary("file", Bytes::from_static(b"hello world"))
                .file_name("hi.txt")
                .content_type("text/plain"),
        ];
        let form = Form::with_boundary("testbound", parts).unwrap();

        let content_type = form.content_type();
        let body: Bytes = collect(form).into();

        let mut req = Request::new(());
        *req.method_mut() = Method::POST;
        req.headers_mut().insert(CONTENT_TYPE, content_type);

        let stream = futures_util::stream::once(async { Ok::<_, Infallible>(body) });
        let mut mp = pin!(multipart(&req, stream).unwrap());

        // Field 1: "name" = "alice"
        {
            let mut f1 = mp.try_next().now_or_never().unwrap().unwrap().unwrap();
            assert_eq!(f1.name(), Some("name"));
            assert_eq!(
                f1.try_next().now_or_never().unwrap().unwrap().unwrap().as_ref(),
                b"alice"
            );
            assert!(f1.try_next().now_or_never().unwrap().unwrap().is_none());
        }

        // Field 2: "file"
        {
            let mut f2 = mp.try_next().now_or_never().unwrap().unwrap().unwrap();
            assert_eq!(f2.name(), Some("file"));
            assert_eq!(f2.file_name(), Some("hi.txt"));
            assert_eq!(f2.headers().get(CONTENT_TYPE).unwrap().as_bytes(), b"text/plain");
            assert_eq!(
                f2.try_next().now_or_never().unwrap().unwrap().unwrap().as_ref(),
                b"hello world"
            );
            assert!(f2.try_next().now_or_never().unwrap().unwrap().is_none());
        }

        // No more fields.
        assert!(mp.try_next().now_or_never().unwrap().unwrap().is_none());
    }

    #[test]
    fn multi_part_delimiters() {
        let form = Form::with_boundary(
            "sep",
            vec![Part::text("a", "1"), Part::text("b", "2"), Part::text("c", "3")],
        )
        .unwrap();
        let body = collect(form);
        // Each single-char value → Content-Length: 1.
        let expected = b"--sep\r\n\
            Content-Disposition: form-data; name=\"a\"\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 1\r\n\r\n1\r\n\
            --sep\r\n\
            Content-Disposition: form-data; name=\"b\"\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 1\r\n\r\n2\r\n\
            --sep\r\n\
            Content-Disposition: form-data; name=\"c\"\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 1\r\n\r\n3\r\n\
            --sep--\r\n";
        assert_eq!(body, expected);
    }

    #[test]
    fn with_boundary_rejects_empty() {
        assert!(matches!(Form::with_boundary("", vec![]), Err(MultipartError::Boundary)));
    }

    #[test]
    fn with_boundary_rejects_newline() {
        assert!(matches!(
            Form::with_boundary("ab\ncd", vec![]),
            Err(MultipartError::Boundary)
        ));
        assert!(matches!(
            Form::with_boundary("ab\rcd", vec![]),
            Err(MultipartError::Boundary)
        ));
    }

    #[test]
    fn content_type_header() {
        let form = Form::with_boundary("myboundary", vec![]).unwrap();
        assert_eq!(
            form.content_type().as_bytes(),
            b"multipart/form-data; boundary=myboundary"
        );
    }
}
