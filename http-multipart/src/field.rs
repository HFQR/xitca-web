use core::{cmp, pin::Pin};

use bytes::{Bytes, BytesMut};
use futures_core::stream::Stream;
use http::header::HeaderMap;
use memchr::memmem;

use super::{content_disposition::ContentDisposition, error::MultipartError, Multipart};

pub struct Field<'a, 'b, S> {
    typ: FieldType,
    cp: ContentDisposition,
    multipart: Pin<&'a mut Multipart<'b, S>>,
}

impl<S> Drop for Field<'_, '_, S> {
    fn drop(&mut self) {
        self.multipart.as_mut().project().headers.clear();
    }
}

impl<'a, 'b, S> Field<'a, 'b, S> {
    pub(super) fn new(length: Option<u64>, cp: ContentDisposition, multipart: Pin<&'a mut Multipart<'b, S>>) -> Self {
        let typ = match length {
            Some(len) => FieldType::Fixed(len),
            None => FieldType::StreamBegin,
        };
        Self { typ, cp, multipart }
    }
}

enum FieldType {
    Fixed(u64),
    StreamBegin,
    StreamEnd,
}

impl<S, T, E> Field<'_, '_, S>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
{
    /// The field name found in the [http::header::CONTENT_DISPOSITION] header.
    pub fn name(&self) -> Option<&str> {
        self.cp
            .name_from_headers(self.headers())
            .and_then(|s| std::str::from_utf8(s).ok())
    }

    /// The file name found in the [http::header::CONTENT_DISPOSITION] header.
    pub fn file_name(&self) -> Option<&str> {
        self.cp
            .filename_from_headers(self.headers())
            .and_then(|s| std::str::from_utf8(s).ok())
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.multipart.headers
    }

    pub async fn try_next(&mut self) -> Result<Option<Bytes>, MultipartError<E>> {
        {
            let this = self.multipart.as_mut().project();
            let buf = this.buf;

            // check multipart buffer first and drain it if possible.
            if !buf.is_empty() {
                match self.typ {
                    FieldType::Fixed(0) | FieldType::StreamEnd => return Ok(None),
                    FieldType::Fixed(ref mut len) => {
                        let at = cmp::min(*len, buf.len() as u64);
                        *len -= at;
                        let chunk = buf.split_to(at as usize).freeze();
                        return Ok(Some(chunk));
                    }
                    FieldType::StreamBegin => {
                        let at = try_find_split_idx(buf, this.boundary, &mut self.typ)?;
                        return Ok(Some(buf.split_to(at).freeze()));
                    }
                }
            }
        }

        // multipart buffer is empty. read more from stream.
        let item = self.multipart.as_mut().try_read_stream().await?;

        let this = self.multipart.as_mut().project();
        let buf = this.buf;

        // try to deal with the read bytes in place before extend to multipart buffer.
        match self.typ {
            FieldType::Fixed(0) => {
                buf.extend_from_slice(item.as_ref());
                Ok(None)
            }
            FieldType::Fixed(ref mut len) => {
                let chunk = item.as_ref();
                let at = cmp::min(*len, chunk.len() as u64);
                *len -= at;
                let bytes = split_bytes(item, at as usize, buf);
                Ok(Some(bytes))
            }
            FieldType::StreamBegin => {
                let at = try_find_split_idx(&item, this.boundary, &mut self.typ)?;
                let bytes = split_bytes(item, at, buf);
                Ok(Some(bytes))
            }
            FieldType::StreamEnd => Ok(None),
        }
    }
}

fn try_find_split_idx<T, E>(item: &T, boundary: &[u8], typ: &mut FieldType) -> Result<usize, MultipartError<E>>
where
    T: AsRef<[u8]>,
{
    let item = item.as_ref();
    match memmem::find(item, boundary) {
        Some(idx) => {
            *typ = FieldType::StreamEnd;
            idx.checked_sub(4).ok_or(MultipartError::Boundary)
        }
        None => Ok(item.len()),
    }
}

fn split_bytes<T>(item: T, at: usize, buf: &mut BytesMut) -> Bytes
where
    T: AsRef<[u8]> + 'static,
{
    match try_downcast_to_bytes(item) {
        Ok(mut item) => {
            let bytes = item.split_to(at);
            buf.extend_from_slice(item.as_ref());
            bytes
        }
        Err(item) => {
            let chunk = item.as_ref();
            let bytes = Bytes::copy_from_slice(&chunk[..at]);
            buf.extend_from_slice(&chunk[at..]);
            bytes
        }
    }
}

fn try_downcast_to_bytes<T: 'static>(item: T) -> Result<Bytes, T> {
    use std::any::Any;

    let item = &mut Some(item);
    match (item as &mut dyn Any).downcast_mut::<Option<Bytes>>() {
        Some(bytes) => Ok(bytes.take().unwrap()),
        None => Err(item.take().unwrap()),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn downcast_bytes() {
        let bytes = Bytes::new();
        assert!(try_downcast_to_bytes(bytes).is_ok());
        let bytes = Vec::<u8>::new();
        assert!(try_downcast_to_bytes(bytes).is_err());
    }
}
