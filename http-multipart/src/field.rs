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

// Tracks progress through the `\r\n--` prefix of a multipart inter-field
// boundary delimiter.  Each variant records how many bytes have been
// consecutively matched at a chunk boundary.
#[derive(Copy, Clone)]
pub(super) enum DelimiterState {
    Cr,           // matched `\r`
    CrLf,         // matched `\r\n`
    CrLfDash,     // matched `\r\n-`
    CrLfDashDash, // matched `\r\n--`; partial boundary name follows in buf
}

impl DelimiterState {
    // Advance the state by one byte of `\r\n--`.
    // Returns `None` if `byte` breaks the match.
    fn step(self, byte: u8) -> Option<Self> {
        match (self, byte) {
            (Self::Cr, b'\n') => Some(Self::CrLf),
            (Self::CrLf, b'-') => Some(Self::CrLfDash),
            (Self::CrLfDash, b'-') => Some(Self::CrLfDashDash),
            _ => None,
        }
    }

    // Find the longest suffix of `bytes` that is a prefix of `\r\n--`.
    // Scans the last 3 bytes (the longest prefix without `--`), restarting
    // the match each time a fresh `\r` is encountered.
    fn from_tail(bytes: &[u8]) -> Option<Self> {
        let tail = &bytes[bytes.len().saturating_sub(3)..];
        let mut state: Option<Self> = None;
        for &byte in tail {
            state = if byte == b'\r' {
                Some(Self::Cr) // start or restart on \r
            } else {
                state.and_then(|s| s.step(byte))
            };
        }
        state
    }

    // Number of `\r\n--` bytes represented by this state.
    fn matched(self) -> usize {
        match self {
            Self::Cr => 1,
            Self::CrLf => 2,
            Self::CrLfDash => 3,
            Self::CrLfDashDash => 4,
        }
    }
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
                    FieldDecoder::StreamBegin | FieldDecoder::StreamDelimiter => {
                        if let Some(at) = self.decoder.try_find_split_idx(buf, multipart.boundary)? {
                            return Ok(Some(buf.split_to(at).freeze()));
                        }
                    }
                }
            }

            match &mut self.decoder {
                // try_find_split_idx may mutate the decoder state to stream end.
                // check it before reading more data from stream
                FieldDecoder::StreamEnd => {
                    *multipart.pending_field = false;
                    return Ok(None);
                }
                decoder => {
                    // multipart buffer is empty. read more from stream.
                    let item = self.multipart.as_mut().try_read_stream().await?;

                    let multipart = self.multipart.as_mut().project();
                    let buf = multipart.buf;

                    // try to deal with the read bytes in place before extend to multipart buffer.
                    match decoder {
                        FieldDecoder::Fixed(0) => {
                            buf.extend_from_slice(item.as_ref());
                            *multipart.pending_field = false;
                            return Ok(None);
                        }
                        FieldDecoder::Fixed(len) => {
                            let chunk = item.as_ref();
                            let at = cmp::min(*len, chunk.len() as u64);
                            *len -= at;
                            let bytes = split_bytes(item, at as usize, buf);
                            return Ok(Some(bytes));
                        }
                        FieldDecoder::StreamBegin => {
                            match self.decoder.try_find_split_idx(&item, multipart.boundary)? {
                                Some(at) => {
                                    let bytes = split_bytes(item, at, buf);
                                    return Ok(Some(bytes));
                                }
                                None => buf.extend_from_slice(item.as_ref()),
                            }
                        }
                        // partial delimiter bytes are already in buf; combine with new data
                        // before searching so that a split "\r\n--" is not missed.
                        FieldDecoder::StreamDelimiter => buf.extend_from_slice(item.as_ref()),
                        FieldDecoder::StreamEnd => unreachable!("outter match covered branch already"),
                    }
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
                // No "\r\n--" in this item.  Use DelimiterState::step to check
                // whether the tail is a partial prefix of "\r\n--" so those bytes
                // can be kept in the buffer for the next read to complete the match.
                match DelimiterState::from_tail(item) {
                    Some(state) if state.matched() >= item.len() => {
                        // Entire item is a potential partial prefix; need more data.
                        *self = FieldDecoder::StreamDelimiter;
                        Ok(None)
                    }
                    Some(state) => {
                        *self = FieldDecoder::StreamDelimiter;
                        Ok(Some(item.len() - state.matched()))
                    }
                    None => {
                        *self = FieldDecoder::StreamBegin;
                        Ok(Some(item.len()))
                    }
                }
            }
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

    #[test]
    fn delimiter_state_step() {
        // happy path: \r\n--
        let s = DelimiterState::Cr;
        let s = s.step(b'\n').unwrap();
        assert!(matches!(s, DelimiterState::CrLf));
        let s = s.step(b'-').unwrap();
        assert!(matches!(s, DelimiterState::CrLfDash));
        let s = s.step(b'-').unwrap();
        assert!(matches!(s, DelimiterState::CrLfDashDash));

        // mismatch breaks the chain
        assert!(DelimiterState::Cr.step(b'x').is_none());
        assert!(DelimiterState::CrLf.step(b'x').is_none());
        assert!(DelimiterState::CrLfDash.step(b'x').is_none());
    }

    #[test]
    fn delimiter_state_from_tail() {
        assert!(matches!(DelimiterState::from_tail(b"abc\r"), Some(DelimiterState::Cr)));
        assert!(matches!(
            DelimiterState::from_tail(b"abc\r\n"),
            Some(DelimiterState::CrLf)
        ));
        assert!(matches!(
            DelimiterState::from_tail(b"abc\r\n-"),
            Some(DelimiterState::CrLfDash)
        ));
        // \r\n-- contains "--" so it is caught by the memmem path, not from_tail;
        // from_tail only sees it when the entire item is that prefix.
        assert!(matches!(
            DelimiterState::from_tail(b"\r\n-"),
            Some(DelimiterState::CrLfDash)
        ));
        // restart on a second \r
        assert!(matches!(
            DelimiterState::from_tail(b"\r\r\n"),
            Some(DelimiterState::CrLf)
        ));
        // no match
        assert!(DelimiterState::from_tail(b"abcxyz").is_none());
        assert!(DelimiterState::from_tail(b"").is_none());
    }
}
