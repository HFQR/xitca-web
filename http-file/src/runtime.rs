use core::future::Future;

use std::{io, path::PathBuf, time::SystemTime};

use bytes::BytesMut;

pub trait AsyncFs: Clone {
    type File: ChunkRead;
    type OpenFuture: Future<Output = io::Result<Self::File>>;

    fn open(&self, path: PathBuf) -> Self::OpenFuture;
}

pub trait Meta {
    fn modified(&mut self) -> Option<SystemTime>;

    fn len(&self) -> u64;
}

pub trait ChunkRead: Meta + Sized {
    type Future: Future<Output = io::Result<Option<(Self, BytesMut, usize)>>>;

    fn next(self, buf: BytesMut) -> Self::Future;
}

#[cfg(feature = "tokio")]
pub(crate) use tokio_impl::*;

#[cfg(feature = "tokio")]
mod tokio_impl {
    use tokio::{fs::File, io::AsyncReadExt};

    use super::*;

    #[derive(Clone)]
    pub struct TokioFs;

    impl AsyncFs for TokioFs {
        type File = TokioFile;
        type OpenFuture = impl Future<Output = io::Result<Self::File>> + Send;

        fn open(&self, path: PathBuf) -> Self::OpenFuture {
            async {
                tokio::task::spawn_blocking(move || {
                    let file = std::fs::File::open(path)?;
                    let meta = file.metadata()?;
                    let modified_time = meta.modified().ok();
                    let len = meta.len();
                    Ok(TokioFile {
                        file: file.into(),
                        modified_time,
                        len,
                    })
                })
                .await
                .unwrap()
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
        type Future = impl Future<Output = io::Result<Option<(Self, BytesMut, usize)>>> + Send;

        fn next(mut self, mut buf: BytesMut) -> Self::Future {
            async {
                let n = self.file.read_buf(&mut buf).await?;

                if n == 0 {
                    Ok(None)
                } else {
                    Ok(Some((self, buf, n)))
                }
            }
        }
    }
}

#[cfg(feature = "tokio-uring")]
pub(crate) use tokio_uring_impl::*;

#[cfg(feature = "tokio-uring")]
mod tokio_uring_impl {
    use tokio_uring::fs::File;

    use super::*;

    #[derive(Clone)]
    pub struct TokioUringFs;

    impl AsyncFs for TokioUringFs {
        type File = TokioUringFile;
        type OpenFuture = impl Future<Output = io::Result<Self::File>> + Send;

        fn open(&self, path: PathBuf) -> Self::OpenFuture {
            async {
                let (file, modified_time, len) = tokio::task::spawn_blocking(move || {
                    let file = std::fs::File::open(path)?;
                    let meta = file.metadata()?;
                    let modified_time = meta.modified().ok();
                    let len = meta.len();
                    Ok::<_, io::Error>((file, modified_time, len))
                })
                .await
                .unwrap()?;

                Ok(TokioUringFile {
                    file: File::from_std(file),
                    pos: 0,
                    modified_time,
                    len,
                })
            }
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
        type Future = impl Future<Output = io::Result<Option<(Self, BytesMut, usize)>>>;

        fn next(mut self, buf: BytesMut) -> Self::Future {
            async {
                let (res, buf) = self.file.read_at(buf, self.pos).await;
                let n = res?;
                if n == 0 {
                    Ok(None)
                } else {
                    self.pos += n as u64;
                    Ok(Some((self, buf, n)))
                }
            }
        }
    }
}
