//! Completion-based async IO wrapper for native-tls TLS streams.

use core::{cell::RefCell, fmt};

use std::{io, net::Shutdown};

use native_tls_crate::{HandshakeError, TlsAcceptor, TlsConnector};

use xitca_io::{
    bytes::BytesMut,
    io::{AsyncBufRead, AsyncBufWrite, BoundedBuf, BoundedBufMut},
};

use crate::bridge::{self, SyncBridge};

/// A TLS stream using native-tls with completion-based async IO.
///
/// Supports concurrent read + write from separate tasks. Concurrent read + read
/// or write + write will panic.
pub struct TlsStream<Io> {
    io: Io,
    session: RefCell<Session>,
}

struct Session {
    tls: native_tls_crate::TlsStream<SyncBridge>,
    /// Taken by the read path. Carries network data across await points.
    read_buf: Option<BytesMut>,
    /// Taken by the write path. Serves as a concurrent-write guard.
    write_buf: Option<BytesMut>,
}

const POLL_TO_COMPLETE: &str = "previous call to future dropped before polling to completion";

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
            Ok(tls) => return Ok(Self::from_tls(io, tls)),
            Err(HandshakeError::WouldBlock(mid)) => mid,
            Err(HandshakeError::Failure(e)) => return Err(Error::Tls(e)),
        };

        loop {
            bridge::drain_write_buf(&io, mid.get_mut()).await.map_err(Error::Io)?;
            bridge::fill_read_buf(&io, mid.get_mut()).await.map_err(Error::Io)?;

            match mid.handshake() {
                Ok(tls) => return Ok(Self::from_tls(io, tls)),
                Err(HandshakeError::WouldBlock(m)) => mid = m,
                Err(HandshakeError::Failure(e)) => return Err(Error::Tls(e)),
            }
        }
    }

    fn from_tls(io: Io, tls: native_tls_crate::TlsStream<SyncBridge>) -> Self {
        TlsStream {
            io,
            session: RefCell::new(Session {
                tls,
                read_buf: Some(BytesMut::new()),
                write_buf: Some(BytesMut::new()),
            }),
        }
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
        let mut session = self.session.borrow_mut();
        let mut read_buf = session.read_buf.take().expect(POLL_TO_COMPLETE);

        // Return previously fetched network data to bridge.
        session.tls.get_mut().read_buf.unsplit(read_buf);

        let res = loop {
            match io::Read::read(&mut session.tls, buf) {
                Ok(n) => {
                    let proto_data = session.tls.get_mut().write_buf.split();
                    drop(session);

                    let drain_res = bridge::drain_split(&self.io, proto_data).await;

                    session = self.session.borrow_mut();
                    if let Err(e) = drain_res {
                        break Err(e);
                    }
                    break Ok(n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let proto_data = session.tls.get_mut().write_buf.split();
                    read_buf = bridge::take_read_buf(session.tls.get_mut());
                    drop(session);

                    let drain_res = bridge::drain_split(&self.io, proto_data).await;
                    let (fill_res, b) = bridge::fill_split(&self.io, read_buf).await;
                    read_buf = b;

                    session = self.session.borrow_mut();
                    session.tls.get_mut().read_buf.unsplit(read_buf);

                    drain_res?;
                    if let Err(e) = fill_res {
                        break Err(e);
                    }
                }
                Err(e) => break Err(e),
            }
        };

        session.read_buf = Some(BytesMut::new());
        res
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
        let mut session = self.session.borrow_mut();
        let write_buf = session.write_buf.take().expect(POLL_TO_COMPLETE);

        let res = loop {
            match io::Write::write(&mut session.tls, buf) {
                Ok(n) => {
                    let ciphertext = session.tls.get_mut().write_buf.split();
                    drop(session);

                    let drain_res = bridge::drain_split(&self.io, ciphertext).await;

                    session = self.session.borrow_mut();
                    if let Err(e) = drain_res {
                        break Err(e);
                    }
                    break Ok(n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let ciphertext = session.tls.get_mut().write_buf.split();
                    drop(session);

                    let drain_res = bridge::drain_split(&self.io, ciphertext).await;

                    session = self.session.borrow_mut();
                    if let Err(e) = drain_res {
                        break Err(e);
                    }
                }
                Err(e) => break Err(e),
            }
        };

        session.write_buf = Some(write_buf);
        res
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
