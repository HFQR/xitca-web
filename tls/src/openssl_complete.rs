//! Completion-based async IO wrapper for OpenSSL TLS streams.

use core::{cell::RefCell, fmt};

use std::{io, net::Shutdown};

pub use openssl::*;

use openssl::ssl::{ErrorCode, Ssl, SslRef, SslStream};

use xitca_io::io::{AsyncBufRead, AsyncBufWrite, BoundedBuf, BoundedBufMut};

use crate::bridge::{self, SyncBridge};

/// A TLS stream using OpenSSL with completion-based async IO.
pub struct TlsStream<Io> {
    io: Io,
    tls: RefCell<SslStream<SyncBridge>>,
}

impl<Io> TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    /// Perform a TLS server-side accept handshake.
    pub async fn accept(ssl: Ssl, io: Io) -> Result<Self, Error> {
        Self::handshake(ssl, io, |tls| tls.accept()).await
    }

    /// Perform a TLS client-side connect handshake.
    pub async fn connect(ssl: Ssl, io: Io) -> Result<Self, Error> {
        Self::handshake(ssl, io, |tls| tls.connect()).await
    }

    async fn handshake<F>(ssl: Ssl, io: Io, mut func: F) -> Result<Self, Error>
    where
        F: FnMut(&mut SslStream<SyncBridge>) -> Result<(), openssl::ssl::Error>,
    {
        let bridge = SyncBridge::new();
        let mut tls = SslStream::new(ssl, bridge)?;

        loop {
            match func(&mut tls) {
                Ok(_) => {
                    return Ok(TlsStream {
                        io,
                        tls: RefCell::new(tls),
                    });
                }
                Err(ref e) if e.code() == ErrorCode::WANT_READ => {
                    bridge::drain_write_buf(&io, tls.get_mut()).await.map_err(Error::Io)?;
                    bridge::fill_read_buf(&io, tls.get_mut()).await.map_err(Error::Io)?;
                }
                Err(ref e) if e.code() == ErrorCode::WANT_WRITE => {
                    bridge::drain_write_buf(&io, tls.get_mut()).await.map_err(Error::Io)?;
                }
                Err(e) => return Err(Error::Tls(e)),
            }
        }
    }

    /// Acquire a reference to the SSL session.
    pub fn session(&self) -> &SslRef {
        let tls = self.tls.borrow();
        // SAFETY: SslRef points into the heap-allocated OpenSSL context, not
        // the RefCell guard. The context lives as long as self.
        unsafe { &*(tls.ssl() as *const SslRef) }
    }
}

impl<Io> AsyncBufRead for TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    async fn read<B>(&self, mut buf: B) -> (io::Result<usize>, B)
    where
        B: BoundedBufMut,
    {
        let init = buf.bytes_init();
        let total = buf.bytes_total();
        let spare = unsafe { core::slice::from_raw_parts_mut(buf.stable_mut_ptr().add(init), total - init) };

        let res = self.read_tls(spare).await;

        if let Ok(n) = &res {
            unsafe { buf.set_init(init + n) };
        }

        (res, buf)
    }
}

impl<Io> TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    async fn read_tls(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut tls = self.tls.borrow_mut();
            match io::Read::read(&mut *tls, buf) {
                Ok(n) => {
                    let write_data = tls.get_mut().write_buf.split();
                    drop(tls);
                    bridge::drain_split(&self.io, write_data).await?;
                    return Ok(n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let write_data = tls.get_mut().write_buf.split();
                    let read_buf = bridge::take_read_buf(tls.get_mut());
                    drop(tls);

                    bridge::drain_split(&self.io, write_data).await?;
                    let read_buf = bridge::fill_split(&self.io, read_buf).await?;

                    self.tls.borrow_mut().get_mut().read_buf.unsplit(read_buf);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl<Io> AsyncBufWrite for TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    async fn write<B>(&self, buf: B) -> (io::Result<usize>, B)
    where
        B: BoundedBuf,
    {
        let data = buf.chunk();
        let res = self.write_tls(data).await;
        (res, buf)
    }

    async fn shutdown(&self, _direction: Shutdown) -> io::Result<()> {
        Ok(())
    }
}

impl<Io> TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    async fn write_tls(&self, buf: &[u8]) -> io::Result<usize> {
        loop {
            let mut tls = self.tls.borrow_mut();
            match io::Write::write(&mut *tls, buf) {
                Ok(n) => {
                    let write_data = tls.get_mut().write_buf.split();
                    drop(tls);
                    bridge::drain_split(&self.io, write_data).await?;
                    return Ok(n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let write_data = tls.get_mut().write_buf.split();
                    drop(tls);
                    bridge::drain_split(&self.io, write_data).await?;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Collection of OpenSSL error types.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Tls(openssl::ssl::Error),
}

impl From<openssl::error::ErrorStack> for Error {
    fn from(e: openssl::error::ErrorStack) -> Self {
        Self::Tls(e.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => fmt::Display::fmt(e, f),
            Self::Tls(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for Error {}
