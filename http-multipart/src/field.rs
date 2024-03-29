use core::{cmp, pin::Pin};

use bytes::{Bytes, BytesMut};
use futures_core::stream::Stream;
use http::header::HeaderMap;
use memchr::memmem;

use super::{
    content_disposition::ContentDisposition,
    error::{MultipartError, PayloadError},
    Multipart,
};

pub struct Field<'a, S> {
    decoder: FieldDecoder,
    cp: ContentDisposition,
    multipart: Pin<&'a mut Multipart<S>>,
}

impl<S> Drop for Field<'_, S> {
    fn drop(&mut self) {
        self.multipart.as_mut().project().headers.clear();
    }
}

impl<'a, S> Field<'a, S> {
    pub(super) fn new(length: Option<u64>, cp: ContentDisposition, multipart: Pin<&'a mut Multipart<S>>) -> Self {
        let typ = match length {
            Some(len) => FieldDecoder::Fixed(len),
            None => FieldDecoder::StreamBegin,
        };
        Self {
            decoder: typ,
            cp,
            multipart,
        }
    }
}

#[derive(Default)]
pub(super) enum FieldDecoder {
    Fixed(u64),
    #[default]
    StreamBegin,
    StreamPossibleEnd,
    StreamEnd,
}

impl<S, T, E> Field<'_, S>
where
    S: Stream<Item = Result<T, E>>,
    T: AsRef<[u8]> + 'static,
    E: Into<PayloadError>,
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

    pub async fn try_next(&mut self) -> Result<Option<Bytes>, MultipartError> {
        loop {
            let multipart = self.multipart.as_mut().project();
            let buf = multipart.buf;

            // check multipart buffer first and drain it if possible.
            if !buf.is_empty() {
                match self.decoder {
                    FieldDecoder::Fixed(0) | FieldDecoder::StreamEnd => {
                        *multipart.pending_field = false;
                        return Ok(None);
                    }
                    FieldDecoder::Fixed(ref mut len) => {
                        let at = cmp::min(*len, buf.len() as u64);
                        *len -= at;
                        let chunk = buf.split_to(at as usize).freeze();
                        return Ok(Some(chunk));
                    }
                    FieldDecoder::StreamBegin | FieldDecoder::StreamPossibleEnd => {
                        if let Some(at) = self.decoder.try_find_split_idx(buf, multipart.boundary)? {
                            return Ok(Some(buf.split_to(at).freeze()));
                        }
                    }
                }
            }

            // multipart buffer is empty. read more from stream.
            let item = self.multipart.as_mut().try_read_stream().await?;

            let multipart = self.multipart.as_mut().project();
            let buf = multipart.buf;

            // try to deal with the read bytes in place before extend to multipart buffer.
            match self.decoder {
                FieldDecoder::Fixed(0) => {
                    buf.extend_from_slice(item.as_ref());
                    *multipart.pending_field = false;
                    return Ok(None);
                }
                FieldDecoder::Fixed(ref mut len) => {
                    let chunk = item.as_ref();
                    let at = cmp::min(*len, chunk.len() as u64);
                    *len -= at;
                    let bytes = split_bytes(item, at as usize, buf);
                    return Ok(Some(bytes));
                }
                FieldDecoder::StreamBegin => match self.decoder.try_find_split_idx(&item, multipart.boundary)? {
                    Some(at) => {
                        let bytes = split_bytes(item, at, buf);
                        return Ok(Some(bytes));
                    }
                    None => buf.extend_from_slice(item.as_ref()),
                },
                // possible boundary where partial boundary is already in the the buffer. so extend to it and check again.
                FieldDecoder::StreamPossibleEnd => buf.extend_from_slice(item.as_ref()),
                FieldDecoder::StreamEnd => {
                    *multipart.pending_field = false;
                    return Ok(None);
                }
            }
        }
    }
}

impl FieldDecoder {
    pub(super) fn try_find_split_idx<T>(&mut self, item: &T, boundary: &[u8]) -> Result<Option<usize>, MultipartError>
    where
        T: AsRef<[u8]>,
    {
        let item = item.as_ref();
        match memmem::find(item, super::DOUBLE_HYPHEN) {
            Some(idx) => {
                let start = idx + super::DOUBLE_HYPHEN.len();
                let length = cmp::min(item.len() - start, boundary.len());
                let end = start + length;

                let slice = &item[start..end];

                // not boundary so split till end offset.
                if !boundary.starts_with(slice) {
                    return Ok(Some(end));
                }

                // possible boundary but no full view yet.
                if boundary.len() > slice.len() {
                    *self = FieldDecoder::StreamPossibleEnd;
                    return Ok(None);
                }

                *self = FieldDecoder::StreamEnd;

                let idx = idx.checked_sub(2).ok_or(MultipartError::Boundary)?;
                Ok(Some(idx))
            }
            None => Ok(Some(item.len())),
        }
    }
}

fn split_bytes<T>(item: T, at: usize, buf: &mut BytesMut) -> Bytes
where
    T: AsRef<[u8]> + 'static,
{
    match try_downcast_to_bytes(item) {
        Ok(mut item) => {
            if item.len() == at {
                return item;
            }
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
