use core::{
    future::Future,
    ops::DerefMut,
    pin::Pin,
    task::{Context, Poll},
};

use std::io;

pub use rustls_crate::*;

use xitca_io::io::{AsyncIo, Interest, Ready};

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
    /// acquire a reference to the session type. Typically either [ClientConnection] or [ServerConnection]
    ///
    /// [ClientConnection]: rustls::ClientConnection
    /// [ServerConnection]: rustls::ServerConnection
    pub fn session(&self) -> &C {
        &self.conn
    }

    /// finish handshake with given io and connection type.
    /// # Examples:
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use xitca_io::net::TcpStream;
    /// use xitca_tls::rustls::{pki_types::ServerName, ClientConfig, ClientConnection, TlsStream};
    ///
    /// async fn client_connect(io: TcpStream, cfg: Arc<ClientConfig>, server_name: ServerName<'static>) {
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
    #[inline]
    fn ready(&self, interest: Interest) -> impl Future<Output = io::Result<Ready>> + Send {
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

fn write_with<C, Io, S, F>(stream: &mut TlsStream<C, Io>, mut func: F) -> io::Result<usize>
where
    Io: AsyncIo,
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
    F: for<'r> FnMut(&mut Writer<'r>) -> io::Result<usize>,
{
    loop {
        match func(&mut stream.conn.writer())? {
            // when rustls writer write 0 it means either the input buffer is empty or it's internal
            // buffer is full. check the condition and flush the io.
            0 if stream.conn.wants_write() => io::Write::flush(stream)?,
            n => return Ok(n),
        }
    }
}
