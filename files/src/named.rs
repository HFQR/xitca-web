use std::{
    fmt,
    fs::Metadata,
    io, mem,
    path::{Path, PathBuf},
    time::SystemTime,
};

use httpdate::HttpDate;
use mime::Mime;
use xitca_http::{
    body::ResponseBody,
    http::{
        header::{CONTENT_LENGTH, CONTENT_TYPE},
        HeaderValue, IntoResponse, Request, Response, StatusCode,
    },
};

use crate::{chunked::new_chunked_read, utf8::equiv_utf8_text};

#[cfg(not(feature = "io-uring"))]
pub(crate) use std::fs::File;
#[cfg(feature = "io-uring")]
pub(crate) use tokio_uring::fs::File;

pub struct NamedFile {
    path: PathBuf,
    status: StatusCode,
    pub(crate) file: File,
    pub(crate) md: Metadata,
    modified: Option<SystemTime>,
    pub(crate) content_type: Mime,
    use_etag: bool,
    use_last_modified: bool,
    use_content_disposition: bool,
    prefer_utf8: bool,
}

impl fmt::Debug for NamedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NamedFile")
            .field("path", &self.path)
            .field(
                "file",
                #[cfg(feature = "io-uring")]
                {
                    &"File"
                },
                #[cfg(not(feature = "io-uring"))]
                {
                    &self.file
                },
            )
            .finish()
    }
}

impl NamedFile {
    /// Attempts to open a file asynchronously in read-only mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use xitca_files::NamedFile;
    ///
    /// # async fn open() {
    /// let file = NamedFile::open("foo.txt").await.unwrap();
    /// # }
    /// ```
    pub async fn open<P: AsRef<Path>>(path: P) -> io::Result<NamedFile> {
        let file = {
            #[cfg(not(feature = "io-uring"))]
            {
                File::open(&path)?
            }

            #[cfg(feature = "io-uring")]
            {
                File::open(&path).await?
            }
        };

        let path = path.as_ref().to_path_buf();

        // Get the name of the file and use it to construct default Content-Type
        // and Content-Disposition values
        let content_type = mime_guess::from_path(&path).first_or_octet_stream();

        let md = {
            #[cfg(not(feature = "io-uring"))]
            {
                file.metadata()?
            }

            #[cfg(feature = "io-uring")]
            {
                use std::os::unix::prelude::{AsRawFd, FromRawFd};

                let fd = file.as_raw_fd();

                // TODO: Remove this.
                // SAFETY: fd is borrowed and lives longer than the unsafe block.
                unsafe {
                    let fs = std::fs::File::from_raw_fd(fd);
                    let res = fs.metadata();
                    mem::forget(fs);
                    res?
                }
            }
        };

        let modified = md.modified().ok();

        Ok(NamedFile {
            path,
            file,
            md,
            modified,
            status: StatusCode::OK,
            content_type,
            use_etag: true,
            use_last_modified: true,
            use_content_disposition: true,
            prefer_utf8: true,
        })
    }

    /// Set response **Status Code**
    pub fn set_status_code(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// Set the MIME Content-Type for serving this file. By default
    /// the Content-Type is inferred from the filename extension.
    #[inline]
    pub fn set_content_type(mut self, mime_type: Mime) -> Self {
        self.content_type = mime_type;
        self
    }

    /// Disable `Content-Disposition` header.
    ///
    /// By default Content-Disposition` header is enabled.
    #[inline]
    pub fn use_content_disposition(mut self, value: bool) -> Self {
        self.use_content_disposition = value;
        self
    }

    /// Specifies whether to use ETag or not.
    ///
    /// Default is true.
    #[inline]
    pub fn use_etag(mut self, value: bool) -> Self {
        self.use_etag = value;
        self
    }

    /// Specifies whether to use Last-Modified or not.
    ///
    /// Default is true.
    #[inline]
    pub fn use_last_modified(mut self, value: bool) -> Self {
        self.use_last_modified = value;
        self
    }

    /// Specifies whether text responses should signal a UTF-8 encoding.
    ///
    /// Default is true.
    #[inline]
    pub fn prefer_utf8(mut self, value: bool) -> Self {
        self.prefer_utf8 = value;
        self
    }

    /// Returns reference to the underlying `File` object.
    #[inline]
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Retrieve the path of this file.
    #[inline]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub fn last_modified(&self) -> Option<HttpDate> {
        self.modified.map(|mtime| mtime.into())
    }

    pub fn into_response<B: Default>(self, req: &mut Request<B>) -> Response<ResponseBody> {
        let len = self.md.len();

        let body = Box::pin(new_chunked_read(len, 0, self.file)) as _;

        let mut res = mem::take(req).into_response(ResponseBody::stream(body));

        let content_type = if self.prefer_utf8 {
            equiv_utf8_text(self.content_type)
        } else {
            self.content_type
        };

        res.headers_mut()
            .append(CONTENT_TYPE, HeaderValue::try_from(content_type.as_ref()).unwrap());
        res.headers_mut().append(CONTENT_LENGTH, HeaderValue::from(len));

        res
    }
}
