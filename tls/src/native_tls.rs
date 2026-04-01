#![allow(clippy::await_holding_refcell_ref)] // clippy is dumb

//! Completion-based async IO wrapper for native-tls TLS streams.

use core::{
    cell::{Ref, RefCell},
    fmt,
};

use std::{io, net::Shutdown};

pub use native_tls_crate::{TlsAcceptor, TlsConnector};

use native_tls_crate::HandshakeError;
use xitca_io::io::{AsyncBufRead, AsyncBufWrite, BoundedBuf, BoundedBufMut};

use crate::bridge::{self, SyncBridge};

/// A TLS stream using native-tls with completion-based async IO.
///
/// Supports one concurrent read + one concurrent write. Concurrent read + read
/// or write + write will panic.
pub struct TlsStream<Io> {
    io: Io,
    tls: RefCell<native_tls_crate::TlsStream<SyncBridge>>,
}

impl<Io> TlsStream<Io>
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    /// Perform a TLS server-side accept handshake.
    pub async fn accept(acceptor: &TlsAcceptor, io: Io) -> Result<Self, Error> {
        let bridge = SyncBridge::new();
        Self::handshake(io, acceptor.accept(bridge)).await
    }

    /// Perform a TLS client-side connect handshake.
    pub async fn connect(connector: &TlsConnector, domain: &str, io: Io) -> Result<Self, Error> {
        let bridge = SyncBridge::new();
        Self::handshake(io, connector.connect(domain, bridge)).await
    }

    async fn handshake(
        io: Io,
        result: Result<native_tls_crate::TlsStream<SyncBridge>, HandshakeError<SyncBridge>>,
    ) -> Result<Self, Error> {
        let mut mid = match result {
            Ok(tls) => {
                return Ok(TlsStream {
                    io,
                    tls: RefCell::new(tls),
                });
            }
            Err(HandshakeError::WouldBlock(mid)) => mid,
            Err(HandshakeError::Failure(e)) => return Err(Error::Tls(e)),
        };

        loop {
            bridge::drain_write_buf(&io, mid.get_mut()).await.map_err(Error::Io)?;
            bridge::fill_read_buf(&io, mid.get_mut()).await.map_err(Error::Io)?;

            match mid.handshake() {
                Ok(tls) => {
                    return Ok(TlsStream {
                        io,
                        tls: RefCell::new(tls),
                    });
                }
                Err(HandshakeError::WouldBlock(m)) => mid = m,
                Err(HandshakeError::Failure(e)) => return Err(Error::Tls(e)),
            }
        }
    }
}

impl<Io> TlsStream<Io> {
    /// Returns the negotiated ALPN protocol, if any.
    pub fn session(&self) -> Ref<'_, native_tls_crate::TlsStream<SyncBridge>> {
        self.tls.borrow()
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

    async fn shutdown(&self, _direction: Shutdown) -> io::Result<()> {
        Ok(())
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
}

/// Collection of native-tls error types.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Tls(native_tls_crate::Error),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<native_tls_crate::Error> for Error {
    fn from(e: native_tls_crate::Error) -> Self {
        Self::Tls(e)
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
