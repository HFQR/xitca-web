mod error;
mod header;

pub use self::error::MultipartError;

use std::{cmp, pin::Pin};

use bytes::{Buf, Bytes, BytesMut};
use futures_util::stream::{Stream, StreamExt};
use http::{header::HeaderMap, Request};
use pin_project_lite::pin_project;

pub fn multipart<Req, B, T, E>(req: Req, body: B) -> Result<Multipart<B>, MultipartError<E>>
where
    Req: std::borrow::Borrow<Request<()>>,
    B: Stream<Item = Result<T, E>>,
    T: Buf,
{
    let headers = req.borrow().headers();

    let boundary = header::boundary(headers)?;

    Ok(Multipart {
        stream: body,
        buf: BytesMut::new(),
        boundary: boundary.into_bytes().into_boxed_slice(),
    })
}

pin_project! {
    pub struct Multipart<S> {
        #[pin]
        stream: S,
        buf: BytesMut,
        boundary: Box<[u8]>,
    }
}

const DOUBLE_DASH: &[u8; 2] = b"--";
const LF: &[u8; 1] = b"\n";
const DOUBLE_CR_LF: &[u8; 4] = b"\r\n\r\n";

impl<S, T, E> Multipart<S>
where
    S: Stream<Item = Result<T, E>>,
    T: Buf,
{
    pub async fn try_next(mut self: Pin<&mut Self>) -> Result<Option<Field<'_, S>>, MultipartError<E>> {
        loop {
            let this = self.as_mut().project();

            if let Some(idx) = twoway::find_bytes(this.buf, LF) {
                // backtrack one byte to exclude CR
                let slice = &this.buf[..idx - 1];

                // empty line. skip.
                if slice.is_empty() {
                    // forward one byte to include LF.
                    this.buf.advance(idx + 1);
                    continue;
                }

                // slice is boundary.
                if &slice[..2] == DOUBLE_DASH {
                    // non last boundary
                    if &slice[2..] == this.boundary.as_ref() {
                        // forward one byte to include CRLF and remove the boundary line.
                        this.buf.advance(idx + 1);

                        let headers = self.as_mut().parse_headers().await?;

                        header::check_headers(&headers)?;

                        let length = header::content_length_opt(&headers)?;

                        return Ok(Some(Field {
                            headers,
                            length,
                            multipart: self,
                        }));
                    }

                    let at = slice.len() - 2;

                    // last boundary.
                    if &slice[2..at] == this.boundary.as_ref() && &slice[at..] == DOUBLE_DASH {
                        this.buf.clear();

                        return Ok(None);
                    }
                }

                return Err(MultipartError::Boundary);
            }

            self.as_mut().try_read_stream_to_buf().await?;
        }
    }

    async fn parse_headers(mut self: Pin<&mut Self>) -> Result<HeaderMap, MultipartError<E>> {
        loop {
            let this = self.as_mut().project();

            if let Some(idx) = twoway::find_bytes(this.buf, DOUBLE_CR_LF) {
                let slice = &this.buf[..idx + 4];
                let headers = header::parse_headers(slice)?;
                this.buf.advance(slice.len());

                return Ok(headers);
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
        match self.project().stream.next().await {
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
}

pub struct Field<'a, S> {
    headers: HeaderMap,
    length: Option<u64>,
    multipart: Pin<&'a mut Multipart<S>>,
}

impl<S, T, E> Field<'_, S>
where
    S: Stream<Item = Result<T, E>>,
    T: Buf,
{
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
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
        req.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("multipart/mixed; boundary=\"abbc761f78ff4d7cb7573b5a23f96ef0\""),
        );

        let body = futures_util::stream::once(async { Ok::<_, ()>(Bytes::copy_from_slice(body)) });

        let multipart = multipart(&req, body).unwrap();

        futures_util::pin_mut!(multipart);

        let mut field = multipart.as_mut().try_next().now_or_never().unwrap().unwrap().unwrap();

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

        let mut field = multipart.as_mut().try_next().now_or_never().unwrap().unwrap().unwrap();

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

        assert!(multipart.as_mut().try_next().now_or_never().unwrap().unwrap().is_none());
        assert!(multipart.try_next().now_or_never().unwrap().is_err());
    }
}
