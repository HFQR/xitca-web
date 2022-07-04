pub(crate) type RustlsConfig = Arc<ServerConfig>;

use std::{
    convert::Infallible,
    error, fmt,
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use rustls_crate::{Error, ServerConfig, ServerConnection, Writer};
use xitca_io::io::{AsyncIo, Interest, Ready};
use xitca_service::{BuildService, Service};

use crate::{http::Version, version::AsVersion};

use super::error::TlsError;

/// A stream managed by rustls for tls read/write.
pub struct TlsStream<Io> {
    io: Io,
    conn: ServerConnection,
}

impl<Io: AsyncIo> TlsStream<Io> {
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

impl<Io> AsVersion for TlsStream<Io> {
    fn as_version(&self) -> Version {
        self.conn
            .alpn_protocol()
            .map(Self::from_alpn)
            .unwrap_or(Version::HTTP_11)
    }
}

/// Rustls Acceptor. Used to accept a unsecure Stream and upgrade it to a TlsStream.
#[derive(Clone)]
pub struct TlsAcceptorService {
    config: Arc<ServerConfig>,
}

impl TlsAcceptorService {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self { config }
    }

    #[inline(never)]
    async fn accept<Io: AsyncIo>(&self, io: Io) -> Result<TlsStream<Io>, RustlsError> {
        let conn = ServerConnection::new(self.config.clone())?;

        let mut stream = TlsStream { io, conn };

        while stream.conn.is_handshaking() {
            while stream.conn.wants_read() {
                stream.io.ready(Interest::READABLE).await?;

                match stream.read_tls() {
                    Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
                    Ok(_) => stream.process_new_packets()?,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
                };
            }

            if stream.conn.wants_write() {
                stream.io.ready(Interest::WRITABLE).await?;
                match stream.write_tls() {
                    Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }

        while let Err(e) = io::Write::flush(&mut stream) {
            if matches!(e.kind(), io::ErrorKind::WouldBlock) {
                stream.io.ready(Interest::WRITABLE).await?;
            } else {
                return Err(e.into());
            }
        }

        Ok(stream)
    }
}

impl BuildService for TlsAcceptorService {
    type Service = TlsAcceptorService;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, _: ()) -> Self::Future {
        let this = self.clone();
        async { Ok(this) }
    }
}

impl<Io: AsyncIo> Service<Io> for TlsAcceptorService {
    type Response = TlsStream<Io>;
    type Error = RustlsError;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, io: Io) -> Self::Future<'_> {
        self.accept(io)
    }
}

impl<Io: AsyncIo> AsyncIo for TlsStream<Io> {
    type ReadyFuture<'f> = impl Future<Output = io::Result<Ready>> where Self: 'f;

    #[inline]
    fn ready(&self, interest: Interest) -> Self::ReadyFuture<'_> {
        self.io.ready(interest)
    }

    fn is_vectored_write(&self) -> bool {
        self.io.is_vectored_write()
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncIo::poll_shutdown(Pin::new(&mut self.get_mut().io), cx)
    }
}

impl<Io: AsyncIo> io::Read for TlsStream<Io> {
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

impl<Io: AsyncIo> io::Write for TlsStream<Io> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write_with(self, |writer| writer.write(buf))
    }

    #[inline]
    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        write_with(self, |writer| writer.write_vectored(bufs))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.conn.writer().flush()?;
        self.io.flush()
    }
}

fn write_with<Io, F>(stream: &mut TlsStream<Io>, func: F) -> io::Result<usize>
where
    Io: AsyncIo,
    F: for<'r> FnOnce(&mut Writer<'r>) -> io::Result<usize>,
{
    // drain remaining data left in tls buffer. this part is not included in current write.
    // (it happens from last try_write call)
    while stream.conn.wants_write() {
        stream.write_tls()?;
    }

    // write to tls buffer and try to send them through wire.
    let n = func(&mut stream.conn.writer())?;

    // there is no guarantee write_tls would be able to send all the n bytes that go into tls buffer.
    // hence the previous while loop try to drain the data.
    stream.write_tls()?;

    // regardless write_tls written how many bytes must advance n bytes that get into tls buffer.
    Ok(n)
}

/// Collection of 'rustls' error types.
pub enum RustlsError {
    Io(io::Error),
    Tls(Error),
}

impl fmt::Debug for RustlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{:?}", e),
            Self::Tls(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl fmt::Display for RustlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Io(ref e) => write!(f, "{}", e),
            Self::Tls(ref e) => write!(f, "{}", e),
        }
    }
}

impl error::Error for RustlsError {}

impl From<io::Error> for RustlsError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Error> for RustlsError {
    fn from(e: Error) -> Self {
        Self::Tls(e)
    }
}

impl From<RustlsError> for TlsError {
    fn from(e: RustlsError) -> Self {
        Self::Rustls(e)
    }
}
