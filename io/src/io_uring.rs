//! async buffer io trait for linux io-uring feature with tokio-uring as runtime.

use core::future::Future;

use std::{io, net::Shutdown};

pub use tokio_uring::buf::{IoBuf, IoBufMut, Slice};

pub trait AsyncBufRead {
    fn read<B>(&self, buf: B) -> impl Future<Output = (io::Result<usize>, B)>
    where
        B: IoBufMut;
}

pub trait AsyncBufWrite {
    fn write<B>(&self, buf: B) -> impl Future<Output = (io::Result<usize>, B)>
    where
        B: IoBuf;

    fn shutdown(&self, direction: Shutdown) -> io::Result<()>;
}

pub async fn write_all<Io, B>(io: &Io, mut buf: B) -> (io::Result<()>, B)
where
    Io: AsyncBufWrite,
    B: IoBuf,
{
    let mut n = 0;
    while n < buf.bytes_init() {
        match io.write(buf.slice(n..)).await {
            (Ok(0), slice) => {
                return (Err(io::ErrorKind::WriteZero.into()), slice.into_inner());
            }
            (Ok(m), slice) => {
                n += m;
                buf = slice.into_inner();
            }
            (Err(e), slice) => {
                return (Err(e), slice.into_inner());
            }
        }
    }
    (Ok(()), buf)
}
