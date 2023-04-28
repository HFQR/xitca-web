use core::{
    ops::DerefMut,
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::io;
use std::io::Write;

pub use rustls::*;

use rustls::{ConnectionCommon, SideData};
use xitca_io::io::{AsyncIo, AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};

/// A stream managed by `rustls` crate for tls read/write.
pub struct TlsStream<C, Io> {
    conn: C,
    io: Io,
}

impl<C, S, Io> TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: io::Read + io::Write,
{
    fn process_new_packets(&mut self) -> io::Result<()> {
        match self.conn.process_new_packets() {
            Ok(_) => Ok(()),
            Err(e) => {
                // In case we have an alert to send describing this error,
                // try a last-gasp write -- but don't predate the primary
                // error.
                let _ = self.write_tls();

                Err(io::Error::new(io::ErrorKind::InvalidData, e))
            }
        }
    }

    fn write_tls(&mut self) -> io::Result<usize> {
        self.conn.write_tls(&mut self.io)
    }

    fn read_tls(&mut self) -> io::Result<usize> {
        self.conn.read_tls(&mut self.io)
    }
}

impl<C, S, Io> TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>> + Unpin,
    S: SideData,
    Io: AsyncIo,
{
    pub fn session(&self) -> &C {
        &self.conn
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

        Ok(TlsStream { io, conn })
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
        self.io.ready(interest)
    }

    #[inline]
    fn poll_ready(&self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        self.io.poll_ready(interest, cx)
    }

    fn is_vectored_write(&self) -> bool {
        self.io.is_vectored_write()
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncIo::poll_shutdown(Pin::new(&mut self.get_mut().io), cx)
    }
}

impl<C, Io, S> io::Read for TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncIo,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.conn.wants_read() {
            let n = self.read_tls()?;

            self.process_new_packets()?;

            if n == 0 {
                break;
            }
        }
        self.conn.reader().read(buf)
    }
}

impl<C, Io, S> io::Write for TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncIo,
{
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write_with(self, |writer| writer.write(buf))
    }

    #[inline]
    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        write_with(self, |writer| writer.write_vectored(bufs))
    }

    fn flush(&mut self) -> io::Result<()> {
        while self.conn.wants_write() {
            if self.write_tls()? == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
        }
        Ok(())
    }
}

fn write_with<C, Io, S, F>(stream: &mut TlsStream<C, Io>, func: F) -> io::Result<usize>
where
    Io: AsyncIo,
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
    F: for<'r> FnOnce(&mut Writer<'r>) -> io::Result<usize>,
{
    stream.flush()?; // flush potential previous buffered write.
    func(&mut stream.conn.writer())
}

impl<C, S, Io> AsyncRead for TlsStream<C, Io>
where
    C: DerefMut<Target = ConnectionCommon<S>> + Unpin,
    S: SideData,
    Io: AsyncIo,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.io.poll_ready(Interest::READABLE, cx))?;
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
        ready!(this.io.poll_ready(Interest::WRITABLE, cx))?;
        match io::Write::write(this, buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.io.poll_ready(Interest::WRITABLE, cx))?;
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
        ready!(this.io.poll_ready(Interest::WRITABLE, cx))?;
        match io::Write::write_vectored(this, bufs) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_vectored_write()
    }
}
