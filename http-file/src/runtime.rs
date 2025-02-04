//! runtime module contains traits for introducing custom async file system impl.

use core::future::Future;

use std::{
    io::{self, SeekFrom},
    path::PathBuf,
    time::SystemTime,
};

use bytes::BytesMut;

/// trait for generic over async file systems.
pub trait AsyncFs {
    type File: ChunkRead + Meta;
    type OpenFuture: Future<Output = io::Result<Self::File>>;

    /// open a file from given path.
    fn open(&self, path: PathBuf) -> Self::OpenFuture;
}

/// trait for generic over file metadata.
pub trait Meta {
    /// the last time when file is modified. optional
    fn modified(&mut self) -> Option<SystemTime>;

    /// the length hint of file.
    fn len(&self) -> u64;

    #[cold]
    #[inline(never)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// trait for async chunk read from file.
pub trait ChunkRead: Sized {
    type SeekFuture<'f>: Future<Output = io::Result<()>> + 'f
    where
        Self: 'f;

    type Future: Future<Output = io::Result<Option<(Self, BytesMut, usize)>>>;

    /// seek file to skip n bytes with given offset.
    fn seek(&mut self, pos: SeekFrom) -> Self::SeekFuture<'_>;

    /// async read of Self and write into given [BytesMut].
    /// return Ok(Some(Self, BytesMut, usize)) after successful read where usize is the byte count
    /// of data written into buffer.
    /// return Ok(None) when self has reached EOF and can not do more read anymore.
    /// return Err(io::Error) when read error occur.
    fn next(self, buf: BytesMut) -> Self::Future;
}

#[cfg(feature = "tokio")]
pub(crate) use tokio_impl::TokioFs;

#[cfg(feature = "tokio")]
mod tokio_impl {
    use core::{
        pin::Pin,
        task::{Context, Poll},
    };

    use tokio::{
        fs::File,
        io::{AsyncReadExt, AsyncSeekExt},
    };

    use super::*;

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    pub struct OpenFuture<F> {
        handle: tokio::task::JoinHandle<F>,
    }

    impl<F> Future for OpenFuture<F> {
        type Output = F;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Pin::new(&mut self.get_mut().handle).poll(cx).map(|res| res.unwrap())
        }
    }

    #[derive(Clone)]
    pub struct TokioFs;

    impl AsyncFs for TokioFs {
        type File = TokioFile;
        type OpenFuture = OpenFuture<io::Result<Self::File>>;

        fn open(&self, path: PathBuf) -> Self::OpenFuture {
            OpenFuture {
                handle: tokio::task::spawn_blocking(move || {
                    let file = std::fs::File::open(path)?;
                    let meta = file.metadata()?;
                    let modified_time = meta.modified().ok();
                    let len = meta.len();
                    Ok(TokioFile {
                        file: file.into(),
                        modified_time,
                        len,
                    })
                }),
            }
        }
    }

    pub struct TokioFile {
        file: File,
        modified_time: Option<SystemTime>,
        len: u64,
    }

    impl Meta for TokioFile {
        fn modified(&mut self) -> Option<SystemTime> {
            self.modified_time
        }

        fn len(&self) -> u64 {
            self.len
        }
    }

    impl ChunkRead for TokioFile {
        type SeekFuture<'f>
            = BoxFuture<'f, io::Result<()>>
        where
            Self: 'f;

        type Future = BoxFuture<'static, io::Result<Option<(Self, BytesMut, usize)>>>;

        fn seek(&mut self, pos: SeekFrom) -> Self::SeekFuture<'_> {
            Box::pin(async move { self.file.seek(pos).await.map(|_| ()) })
        }

        fn next(mut self, mut buf: BytesMut) -> Self::Future {
            Box::pin(async {
                let n = self.file.read_buf(&mut buf).await?;
                if n == 0 {
                    Ok(None)
                } else {
                    Ok(Some((self, buf, n)))
                }
            })
        }
    }
}

#[cfg(feature = "tokio-uring")]
pub(crate) use tokio_uring_impl::TokioUringFs;

#[cfg(feature = "tokio-uring")]
mod tokio_uring_impl {
    use core::{
        future::{ready, Ready},
        pin::Pin,
    };

    use tokio_uring::fs::File;

    use super::*;

    type BoxFuture<'f, T> = Pin<Box<dyn Future<Output = T> + 'f>>;

    #[derive(Clone)]
    pub struct TokioUringFs;

    impl AsyncFs for TokioUringFs {
        type File = TokioUringFile;
        type OpenFuture = BoxFuture<'static, io::Result<Self::File>>;

        fn open(&self, path: PathBuf) -> Self::OpenFuture {
            Box::pin(async {
                let file = File::open(path).await?;

                // SAFETY: fd is borrowed and lives longer than the unsafe block
                let meta = unsafe {
                    use std::os::fd::{AsRawFd, FromRawFd};

                    let file = std::fs::File::from_raw_fd(file.as_raw_fd());
                    let md = file.metadata();
                    // SAFETY: forget the fd before exiting block in success or error case but don't
                    // run destructor (that would close file handle)
                    core::mem::forget(file);
                    md?
                };

                let modified_time = meta.modified().ok();
                let len = meta.len();

                Ok(TokioUringFile {
                    file,
                    pos: 0,
                    modified_time,
                    len,
                })
            })
        }
    }

    pub struct TokioUringFile {
        file: File,
        pos: u64,
        modified_time: Option<SystemTime>,
        len: u64,
    }

    impl Meta for TokioUringFile {
        fn modified(&mut self) -> Option<SystemTime> {
            self.modified_time
        }

        fn len(&self) -> u64 {
            self.len
        }
    }

    impl ChunkRead for TokioUringFile {
        type SeekFuture<'f>
            = Ready<io::Result<()>>
        where
            Self: 'f;

        type Future = BoxFuture<'static, io::Result<Option<(Self, BytesMut, usize)>>>;

        fn seek(&mut self, pos: SeekFrom) -> Self::SeekFuture<'_> {
            let SeekFrom::Start(pos) = pos else {
                unreachable!("ChunkRead::seek only accept pos as SeekFrom::Start variant")
            };
            self.pos += pos;
            ready(Ok(()))
        }

        fn next(mut self, buf: BytesMut) -> Self::Future {
            Box::pin(async {
                let (res, buf) = self.file.read_at(buf, self.pos).await;
                let n = res?;
                if n == 0 {
                    Ok(None)
                } else {
                    self.pos += n as u64;
                    Ok(Some((self, buf, n)))
                }
            })
        }
    }
}
