use std::{cmp, pin::Pin};

use bytes::Bytes;
use futures_core::stream::Stream;
use http::header::HeaderMap;

use super::{content_disposition::ContentDisposition, error::MultipartError, Multipart};

pub struct Field<'a, 'b, S> {
    pub(super) length: Option<u64>,
    pub(super) cp: ContentDisposition,
    pub(super) multipart: Pin<&'a mut Multipart<'b, S>>,
}

impl<S> Drop for Field<'_, '_, S> {
    fn drop(&mut self) {
        self.multipart.as_mut().project().headers.clear();
    }
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
        let item = self.multipart.as_mut().try_read_stream().await?;

        // try to deal with the read bytes in place before extend to multipart buffer.
        match self.length.as_mut() {
            Some(0) => {
                self.multipart.as_mut().buf_extend(item.as_ref());
                Ok(None)
            }
            Some(len) => {
                let chunk = item.as_ref();

                let at = cmp::min(*len, chunk.len() as u64);
                *len -= at;

                let at = at as usize;

                match try_downcast_to_bytes(item) {
                    Ok(mut item) => {
                        let bytes = item.split_to(at);
                        self.multipart.as_mut().buf_extend(item.as_ref());
                        Ok(Some(bytes))
                    }
                    Err(item) => {
                        let chunk = item.as_ref();
                        let bytes = Bytes::copy_from_slice(&chunk[..at]);
                        self.multipart.as_mut().buf_extend(&chunk[at..]);
                        Ok(Some(bytes))
                    }
                }
            }
            None => unimplemented!("multipart field without content length header is not supported yet"),
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
