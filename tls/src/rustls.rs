use core::{
    ops::DerefMut,
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::io;

pub use rustls::*;

use rustls::{ConnectionCommon, SideData, StreamOwned};
use xitca_io::io::{AsyncIo, AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};

/// A stream managed by `rustls` crate for tls read/write.
pub struct TlsStream<C, Io>
where
    Io: AsyncIo,
{
    io: StreamOwned<C, Io>,
}

impl<C, S, Io> TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>> + Unpin,
    S: SideData,
    Io: AsyncIo,
{
    pub fn session(&self) -> &C {
        &self.io.conn
    }

    /// finish handshake with given io and connection type.
    /// # Examples:
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use rustls::{ClientConfig, ClientConnection, ServerName};
    /// use xitca_io::net::TcpStream;
    /// use xitca_tls::rustls::TlsStream;
    ///
    /// async fn client_connect(io: TcpStream, cfg: Arc<ClientConfig>, server_name: ServerName) {
    ///     let conn = ClientConnection::new(cfg, server_name).unwrap();
    ///     let _stream = TlsStream::handshake(io, conn).await.unwrap();
    /// }
    /// ```
    pub async fn handshake(mut io: Io, mut conn: C) -> io::Result<Self> {
        while let Err(e) = conn.complete_io(&mut io) {
            if !matches!(e.kind(), io::ErrorKind::WouldBlock) {
                return Err(e);
            }
            let interest = match (conn.wants_read(), conn.wants_write()) {
                (true, true) => Interest::READABLE | Interest::WRITABLE,
                (true, false) => Interest::READABLE,
                (false, true) => Interest::WRITABLE,
                (false, false) => unreachable!(),
            };
            io.ready(interest).await?;
        }

        Ok(TlsStream {
            io: StreamOwned::new(conn, io),
        })
    }
}

impl<C, Io, S> AsyncIo for TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>> + Unpin,
    S: SideData,
    Io: AsyncIo,
{
    type Future<'f> = Io::Future<'f> where Self: 'f;

    #[inline]
    fn ready(&self, interest: Interest) -> Self::Future<'_> {
        self.io.get_ref().ready(interest)
    }

    #[inline]
    fn poll_ready(&self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        self.io.get_ref().poll_ready(interest, cx)
    }

    fn is_vectored_write(&self) -> bool {
        self.io.get_ref().is_vectored_write()
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncIo::poll_shutdown(Pin::new(self.get_mut().io.get_mut()), cx)
    }
}

impl<C, S, Io> io::Read for TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncIo,
{
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.io, buf)
    }
}

impl<C, S, Io> io::Write for TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncIo,
{
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(&mut self.io, buf)
    }

    #[inline]
    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        io::Write::write_vectored(&mut self.io, bufs)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.io)
    }
}

impl<C, S, Io> AsyncRead for TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>> + Unpin,
    S: SideData,
    Io: AsyncIo,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.io.get_ref().poll_ready(Interest::READABLE, cx))?;
        match io::Read::read(this, buf.initialize_unfilled()) {
            Ok(n) => {
                buf.advance(n);
                Poll::Ready(Ok(()))
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<C, S, Io> AsyncWrite for TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>> + Unpin,
    S: SideData,
    Io: AsyncIo,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.io.get_ref().poll_ready(Interest::WRITABLE, cx))?;
        match io::Write::write(this, buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.io.get_ref().poll_ready(Interest::WRITABLE, cx))?;
        match io::Write::flush(this) {
            Ok(_) => Poll::Ready(Ok(())),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncIo::poll_shutdown(self, cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.io.get_ref().poll_ready(Interest::WRITABLE, cx))?;
        match io::Write::write_vectored(this, bufs) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.io.get_ref().is_vectored_write()
    }
}
