use core::{future::poll_fn, pin::Pin, slice};

use std::{io, net::Shutdown};

use tokio::{
    io::{AsyncWrite, Interest},
    net::{TcpStream, UnixStream},
};

use crate::buf::{BoundedBuf, BoundedBufMut};

use super::{async_buf_read::AsyncBufRead, async_buf_write::AsyncBufWrite};

macro_rules! trait_impl {
    ($ty: ty) => {
        impl AsyncBufRead for $ty {
            #[allow(unsafe_code)]
            async fn read<B>(&self, mut buf: B) -> (io::Result<usize>, B)
            where
                B: BoundedBufMut,
            {
                let init = buf.bytes_init();
                let total = buf.bytes_total();

                // Safety: construct a mutable slice over the spare capacity.
                // try_read writes contiguously from the start of the slice
                // and returns the exact byte count written on Ok(n).
                let spare = unsafe { slice::from_raw_parts_mut(buf.stable_mut_ptr().add(init), total - init) };

                let mut written = 0;

                let res = loop {
                    if written == spare.len() {
                        break Ok(written);
                    }

                    match self.try_read(&mut spare[written..]) {
                        Ok(0) => break Ok(written),
                        Ok(n) => written += n,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            if written > 0 {
                                break Ok(written);
                            }
                            if let Err(e) = self.ready(Interest::READABLE).await {
                                break Err(e);
                            }
                        }
                        Err(e) => break Err(e),
                    }
                };

                // SAFETY: TcpStream::try_read has put written bytes into buf.
                unsafe {
                    buf.set_init(init + written);
                }

                (res, buf)
            }
        }

        impl AsyncBufWrite for $ty {
            async fn write<B>(&self, buf: B) -> (io::Result<usize>, B)
            where
                B: BoundedBuf,
            {
                let data = buf.chunk();

                let mut written = 0;

                let res = loop {
                    if written == data.len() {
                        break Ok(written);
                    }

                    match self.try_write(&data[written..]) {
                        Ok(0) => break Ok(written),
                        Ok(n) => written += n,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            if written > 0 {
                                break Ok(written);
                            }
                            if let Err(e) = self.ready(Interest::WRITABLE).await {
                                break Err(e);
                            }
                        }
                        Err(e) => break Err(e),
                    }
                };

                (res, buf)
            }

            async fn shutdown(mut self, _: Shutdown) -> io::Result<()> {
                poll_fn(|cx| Pin::new(&mut self).poll_shutdown(cx)).await
            }
        }
    };
}

trait_impl!(TcpStream);
trait_impl!(UnixStream);
