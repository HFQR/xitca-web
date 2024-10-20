//! async buffer io trait for linux io-uring feature with tokio-uring as runtime.

use core::future::Future;

use std::{io, net::Shutdown};

use tokio_uring::buf::IoBuf;

pub use tokio_uring::buf::{BoundedBuf, BoundedBufMut, Slice};

pub trait AsyncBufRead {
    fn read<B>(&self, buf: B) -> impl Future<Output = (io::Result<usize>, B)>
    where
        B: BoundedBufMut;
}

pub trait AsyncBufWrite {
    fn write<B>(&self, buf: B) -> impl Future<Output = (io::Result<usize>, B)>
    where
        B: BoundedBuf;

    fn shutdown(&self, direction: Shutdown) -> io::Result<()>;
}

pub async fn write_all<Io, B>(io: &Io, buf: B) -> (io::Result<()>, B)
where
    Io: AsyncBufWrite,
    B: IoBuf,
{
    let mut buf = buf.slice_full();
    while buf.bytes_init() != 0 {
        match io.write(buf).await {
            (Ok(0), slice) => {
                return (Err(io::ErrorKind::WriteZero.into()), slice.into_inner());
            }
            (Ok(n), slice) => buf = slice.slice(n..),
            (Err(e), slice) => {
                return (Err(e), slice.into_inner());
            }
        }
    }
    (Ok(()), buf.into_inner())
}
