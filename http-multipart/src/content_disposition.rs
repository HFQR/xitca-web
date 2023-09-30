use http::header::{HeaderMap, CONTENT_DISPOSITION};
use memchr::memmem;

use super::error::MultipartError;

/// indices of name and filename field of [CONTENT_DISPOSITION]'s header value.
pub struct ContentDisposition {
    name_indice: Option<(usize, usize)>,
    filename_indice: Option<(usize, usize)>,
}

impl ContentDisposition {
    const FORM: &'static [u8; 11] = b"form-data; ";
    const NAME: &'static [u8; 5] = b"name=";
    const FILE_NAME: &'static [u8; 9] = b"filename=";

    pub(super) fn try_from_header<E>(headers: &HeaderMap) -> Result<Self, MultipartError<E>> {
        let header = headers
            .get(&CONTENT_DISPOSITION)
            .ok_or(MultipartError::NoContentDisposition)?
            .as_bytes();

        if !header.starts_with(Self::FORM) {
            return Err(MultipartError::NoContentDisposition);
        }

        Ok(Self::from_slice(header))
    }

    pub(super) fn name_from_headers<'h>(&self, headers: &'h HeaderMap) -> Option<&'h [u8]> {
        let header = headers
            .get(&CONTENT_DISPOSITION)
            .expect("ContentDisposition::try_from_header must make sure header exists");
        self.name(header.as_bytes())
    }

    pub(super) fn filename_from_headers<'h>(&self, headers: &'h HeaderMap) -> Option<&'h [u8]> {
        let header = headers
            .get(&CONTENT_DISPOSITION)
            .expect("ContentDisposition::try_from_header must make sure header exists");
        self.filename(header.as_bytes())
    }

    fn name<'h>(&self, slice: &'h [u8]) -> Option<&'h [u8]> {
        self.name_indice.map(|(start, end)| &slice[start..end])
    }

    fn filename<'h>(&self, slice: &'h [u8]) -> Option<&'h [u8]> {
        self.filename_indice.map(|(start, end)| &slice[start..end])
    }

    fn from_slice(slice: &[u8]) -> Self {
        Self {
            name_indice: Self::indice_from_slice(slice, Self::NAME),
            filename_indice: Self::indice_from_slice(slice, Self::FILE_NAME),
        }
    }

    fn indice_from_slice(haystack: &[u8], needle: &[u8]) -> Option<(usize, usize)> {
        memmem::find(haystack, needle).and_then(|idx| {
            if needle == Self::NAME && idx > 0 && haystack[idx - 1] == b'e' {
                None
            } else {
                let mut start = idx + needle.len();
                let remain = &haystack[start..];
                let mut len = memchr::memchr(b';', remain).unwrap_or(remain.len());

                let remain = &remain[..len];

                // adjust length for quote names.
                if remain.starts_with(b"\"") {
                    start += 1;
                    if let Some(idx) = memchr::memchr(b'"', &remain[1..]) {
                        len = idx;
                    }
                }

                Some((start, start + len))
            }
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_content_disposition_name_only() {
        let slice = br#"form-data; name="my_field""#;
        let cp = ContentDisposition::from_slice(slice);

        let name = cp.name(slice).unwrap();
        assert_eq!(name, b"my_field");
        assert!(cp.filename(slice).is_none());
    }

    #[test]
    fn test_content_disposition_extraction() {
        let slice = br#"form-data; name="my_field"; filename="file abc.txt""#;
        let cp = ContentDisposition::from_slice(slice);
        assert_eq!(cp.name(slice).unwrap(), b"my_field");
        assert_eq!(cp.filename(slice).unwrap(), b"file abc.txt");

        let slice = "form-data; name=\"你好\"; filename=\"file abc.txt\"".as_bytes();
        let cp = ContentDisposition::from_slice(slice);
        assert_eq!(cp.name(slice).unwrap(), "你好".as_bytes());
        assert_eq!(cp.filename(slice).unwrap(), b"file abc.txt");

        let slice = "form-data; name=\"কখগ\"; filename=\"你好.txt\"".as_bytes();
        let cp = ContentDisposition::from_slice(slice);
        assert_eq!(cp.name(slice).unwrap(), "কখগ".as_bytes());
        assert_eq!(cp.filename(slice).unwrap(), "你好.txt".as_bytes());
    }

    #[test]
    fn test_content_disposition_file_name_only() {
        let slice = br#"form-data; filename="file-name.txt""#;
        let cp = ContentDisposition::from_slice(slice);
        assert_eq!(cp.filename(slice).unwrap(), b"file-name.txt");
        assert!(cp.name(slice).is_none());

        let slice = "form-data; filename=\"কখগ-你好.txt\"".as_bytes();
        let cp = ContentDisposition::from_slice(slice);
        assert_eq!(cp.filename(slice).unwrap(), "কখগ-你好.txt".as_bytes());
        assert!(cp.name(slice).is_none());
    }

    #[test]
    fn test_content_disposition_name_unquoted() {
        let slice = br"form-data; name=my_field";
        let cp = ContentDisposition::from_slice(slice);
        assert_eq!(cp.name(slice).unwrap(), b"my_field");
        assert!(cp.filename(slice).is_none());

        let slice = br"form-data; name=my_field; filename=file-name.txt";
        let cp = ContentDisposition::from_slice(slice);
        assert_eq!(cp.name(slice).unwrap(), b"my_field");
        assert_eq!(cp.filename(slice).unwrap(), b"file-name.txt");
    }
}
