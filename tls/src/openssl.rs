#![allow(clippy::await_holding_refcell_ref)] // clippy is dumb

//! Completion-based async IO wrapper for OpenSSL TLS streams.

use core::{
    cell::{Ref, RefCell},
    fmt,
};

use std::{io, net::Shutdown};

pub use openssl::*;

use openssl::ssl::{ErrorCode, Ssl, SslRef, SslStream};
use xitca_io::io::{AsyncBufRead, AsyncBufWrite, BoundedBuf, BoundedBufMut};

use crate::bridge::{self, SyncBridge};

/// A TLS stream using OpenSSL with completion-based async IO.
///
/// Supports one concurrent read + one concurrent write. Concurrent read + read
/// or write + write will panic.
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
                    bridge::drain_write_buf(&io, tls.get_mut()).await.map_err(Error::Io)?;
                    return Ok(TlsStream {
                        io,
                        tls: RefCell::new(tls),
                    });
                }
                Err(ref e) if e.code() == ErrorCode::WANT_WRITE => {
                    bridge::drain_write_buf(&io, tls.get_mut()).await.map_err(Error::Io)?;
                }
                Err(ref e) if e.code() == ErrorCode::WANT_READ => {
                    bridge::drain_write_buf(&io, tls.get_mut()).await.map_err(Error::Io)?;
                    bridge::fill_read_buf(&io, tls.get_mut()).await.map_err(Error::Io)?;
                }
                Err(e) => return Err(Error::Tls(e)),
            }
        }
    }

    /// Acquire a reference to the SSL session.
    pub fn session(&self) -> Ref<'_, SslRef> {
        let tls = self.tls.borrow();
        Ref::map(tls, |tls| tls.ssl())
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
        // SAFETY: only write to spare slice without reading it.
        let spare = unsafe { bridge::spare_capacity_mut(&mut buf) };
        let res = self.read_tls(spare).await;

        if let Ok(n) = &res {
            let init = buf.bytes_init();
            // SAFETY: read_tls writes contiguously from the start of the spare
            // slice, returning exactly n bytes written. init + n is the new
            // initialized boundary.
            unsafe { buf.set_init(init + n) };
        }

        (res, buf)
    }
}

impl<Io> TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    /// Read path: only reads from the IO socket, never writes.
    /// Protocol write data (key updates, alerts) is stashed in
    /// `proto_write_buf` for the write path to flush.
    async fn read_tls(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut tls = self.tls.borrow_mut();

        loop {
            match io::Read::read(&mut *tls, buf) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // Take read_buf — panics if another read is in progress.
                    let mut read_buf = tls.get_mut().take_read_buf();

                    // Prepare read_buf for network fill.
                    let len = read_buf.len();
                    read_buf.reserve(4096);

                    drop(tls);

                    let (res, buf) = self.io.read(read_buf.slice(len..)).await;

                    tls = self.tls.borrow_mut();
                    // Feed new data to bridge for next tls.read() iteration.
                    tls.get_mut().set_read_buf(buf.into_inner());

                    match res {
                        Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                        Ok(_) => {}
                        Err(e) => return Err(e),
                    }
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

    async fn shutdown(mut self, direction: Shutdown) -> io::Result<()> {
        let res = self.tls_shutdown().await;
        let shutdown_res = self.io.shutdown(direction).await;

        res?;
        shutdown_res
    }
}

impl<Io> TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    /// Write path: owns the IO write side exclusively.
    /// Flushes protocol data buffered by the read path before its own ciphertext.
    async fn write_tls(&self, buf: &[u8]) -> io::Result<usize> {
        let mut tls = self.tls.borrow_mut();

        loop {
            match io::Write::write(&mut *tls, buf) {
                Ok(n) => {
                    let buf = tls.get_mut().take_write_buf();
                    drop(tls);

                    let (res, buf) = bridge::drain_write(&self.io, buf).await;

                    tls = self.tls.borrow_mut();
                    tls.get_mut().set_write_buf(buf);

                    return res.map(|_| n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let buf = tls.get_mut().take_write_buf();
                    drop(tls);

                    let (res, buf) = bridge::drain_write(&self.io, buf).await;

                    tls = self.tls.borrow_mut();
                    tls.get_mut().set_write_buf(buf);

                    res?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn tls_shutdown(&mut self) -> io::Result<()> {
        self.write_tls(&[]).await?;

        let tls = self.tls.get_mut();

        loop {
            match tls.shutdown() {
                Ok(ssl::ShutdownResult::Sent) | Ok(ssl::ShutdownResult::Received) => {
                    bridge::drain_write_buf(&self.io, tls.get_mut()).await?;
                    return Ok(());
                }
                Err(ref e) if e.code() == ErrorCode::WANT_WRITE => {
                    bridge::drain_write_buf(&self.io, tls.get_mut()).await?;
                }
                Err(ref e) if e.code() == ErrorCode::WANT_READ => {
                    bridge::drain_write_buf(&self.io, tls.get_mut()).await?;
                    bridge::fill_read_buf(&self.io, tls.get_mut()).await?;
                }
                Err(e) => {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, e));
                }
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
