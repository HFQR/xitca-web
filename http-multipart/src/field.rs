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
    StreamDelimiter,
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
                        // next step would match on decoder and handle field eof
                    }
                    FieldDecoder::Fixed(ref mut len) => {
                        let at = cmp::min(*len, buf.len() as u64);
                        *len -= at;
                        let chunk = buf.split_to(at as usize).freeze();
                        return Ok(Some(chunk));
                    }
                    FieldDecoder::StreamBegin | FieldDecoder::StreamDelimiter => {
                        if let Some(at) = self.decoder.try_find_split_idx(buf, multipart.boundary)? {
                            return Ok(Some(buf.split_to(at).freeze()));
                        }
                    }
                }
            }

            match &mut self.decoder {
                // previoous try_find_split_idx may mutate the decoder state to stream end.
                // lower the field eof here would catch all possible eof state
                FieldDecoder::Fixed(0) | FieldDecoder::StreamEnd => {
                    *multipart.pending_field = false;
                    return Ok(None);
                }
                decoder => {
                    // multipart buffer is empty. read more from stream.
                    let item = self.multipart.as_mut().try_read_stream().await?;

                    let multipart = self.multipart.as_mut().project();
                    let buf = multipart.buf;

                    // try to deal with the read bytes in place to reduce memory footprint.
                    // fall back to multipart buffer memory when streaming parser can't be
                    // determined
                    match decoder {
                        // len == 0 is already handled in the outter match.
                        FieldDecoder::Fixed(len) => {
                            let chunk = item.as_ref();
                            let at = cmp::min(*len, chunk.len() as u64);
                            *len -= at;
                            let bytes = split_bytes(item, at as usize, buf);
                            return Ok(Some(bytes));
                        }
                        FieldDecoder::StreamBegin => {
                            if let Some(at) = self.decoder.try_find_split_idx(&item, multipart.boundary)? {
                                let bytes = split_bytes(item, at, buf);
                                return Ok(Some(bytes));
                            }
                        }
                        // partial delimiter bytes are already in buf; combine with new data
                        // before searching so that a split "\r\n--" is not missed.
                        FieldDecoder::StreamDelimiter => {}
                        FieldDecoder::StreamEnd => unreachable!("outter match covered branch already"),
                    };

                    buf.extend_from_slice(item.as_ref());
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

        match memmem::find(item, super::FIELD_DELIMITER) {
            Some(idx) => {
                let start = idx + super::FIELD_DELIMITER.len();
                let length = cmp::min(item.len() - start, boundary.len());
                let slice = &item[start..start + length];

                // not boundary — yield up to the end of the checked bytes.
                if !boundary.starts_with(slice) {
                    return Ok(Some(start + length));
                }

                // boundary prefix matched but full name not yet visible.
                *self = if boundary.len() > slice.len() {
                    FieldDecoder::StreamDelimiter
                } else {
                    FieldDecoder::StreamEnd
                };

                // caller would split a byte buffer based on returned idx
                // split at 0 is wasteful
                Ok((idx > 0).then_some(idx))
            }
            None => {
                // No "\r\n--" in this item. check whether the tail is a partial prefix
                // of "\r\n--" so those bytes can be kept in the buffer for the next
                // read to complete the match.
                Ok(match potential_boundary_tail(item) {
                    Some(keep) => {
                        *self = FieldDecoder::StreamDelimiter;
                        (keep < item.len()).then_some(item.len() - keep)
                    }
                    None => {
                        *self = FieldDecoder::StreamBegin;
                        Some(item.len())
                    }
                })
            }
        }
    }
}

fn potential_boundary_tail(item: &[u8]) -> Option<usize> {
    let len = item.len();
    item.last()?
        .eq(&b'\r')
        .then_some(1)
        .or_else(|| item[len.saturating_sub(2)..].eq(b"\r\n").then_some(2))
        .or_else(|| item[len.saturating_sub(3)..].eq(b"\r\n-").then_some(3))
}

// split chunked item bytes. determined streamable bytes are returned.
// the rest part is extended onto multipart's internal buffer.
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
