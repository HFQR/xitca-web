use std::{
    fmt,
    fs::Metadata,
    io,
    path::{Path, PathBuf},
};

pub struct NamedFile {
    path: PathBuf,
    pub(crate) file: File,
    pub(crate) md: Metadata,
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

#[cfg(not(feature = "io-uring"))]
pub(crate) use std::fs::File;
#[cfg(feature = "io-uring")]
pub(crate) use tokio_uring::fs::File;

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

        Self::from_file(file, path)
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

    fn from_file<P: AsRef<Path>>(file: File, path: P) -> io::Result<NamedFile> {
        let path = path.as_ref().to_path_buf();

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
                    std::mem::forget(fs);
                    res?
                }
            }
        };

        Ok(NamedFile { path, file, md })
    }
}
