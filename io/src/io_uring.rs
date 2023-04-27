//! async buffer io trait for linux io-uring feature with tokio-uring as runtime.

use core::future::Future;

use std::{io, net::Shutdown};

pub use tokio_uring::buf::{IoBuf, IoBufMut, Slice};

pub trait AsyncBufRead {
    type Future<'f, B>: Future<Output = (io::Result<usize>, B)> + 'f
    where
        Self: 'f,
        B: IoBufMut + 'f;

    fn read<B>(&self, buf: B) -> Self::Future<'_, B>
    where
        B: IoBufMut;
}

pub trait AsyncBufWrite {
    type Future<'f, B>: Future<Output = (io::Result<usize>, B)> + 'f
    where
        Self: 'f,
        B: IoBuf + 'f;

    fn write<B>(&self, buf: B) -> Self::Future<'_, B>
    where
        B: IoBuf;

    fn shutdown(&self, direction: Shutdown) -> io::Result<()>;
}
