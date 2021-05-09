pub use tokio::net::{TcpListener, TcpSocket, TcpStream};

#[cfg(unix)]
pub use tokio::net::{UnixListener, UnixStream};

use std::io;
use std::net;

#[derive(Debug)]
pub(crate) enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

pub enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

pub trait FromStream {
    fn from_stream(stream: Stream) -> Self;
}

impl FromStream for TcpStream {
    fn from_stream(stream: Stream) -> Self {
        match stream {
            Stream::Tcp(tcp) => tcp,
            #[cfg(unix)]
            Stream::Unix(_) => unreachable!("UnixStream can not be casted to TcpStream"),
        }
    }
}

impl FromStream for UnixStream {
    fn from_stream(stream: Stream) -> Self {
        match stream {
            Stream::Tcp(_) => unreachable!("TcpStream can not be casted to UnixStream"),
            #[cfg(unix)]
            Stream::Unix(unix) => unix,
        }
    }
}

/// Helper trait for convert std listener types to tokio types.
/// This is to delay the conversion and make it happen in server thread.
/// Otherwise it would panic.
pub(crate) trait AsListener {
    fn as_listener(&mut self) -> io::Result<Listener>;
}

impl AsListener for Option<net::TcpListener> {
    fn as_listener(&mut self) -> io::Result<Listener> {
        let this = self.take().unwrap();
        this.set_nonblocking(true)?;
        TcpListener::from_std(this).map(Listener::Tcp)
    }
}

#[cfg(unix)]
impl AsListener for Option<std::os::unix::net::UnixListener> {
    fn as_listener(&mut self) -> io::Result<Listener> {
        let this = self.take().unwrap();
        this.set_nonblocking(true)?;
        UnixListener::from_std(this).map(Listener::Unix)
    }
}

impl Listener {
    pub(crate) async fn accept(&self) -> io::Result<Stream> {
        match *self {
            Self::Tcp(ref tcp) => {
                let (stream, _) = tcp.accept().await?;

                // This two way conversion is to deregister stream from the listener thread's poll
                // and re-register it to current thread's poll.
                let stream = stream.into_std()?;
                let stream = TcpStream::from_std(stream)?;
                Ok(Stream::Tcp(stream))
            }
            #[cfg(unix)]
            Self::Unix(ref unix) => {
                let (stream, _) = unix.accept().await?;

                // This two way conversion is to deregister stream from the listener thread's poll
                // and re-register it to current thread's poll.
                let stream = stream.into_std()?;
                let stream = UnixStream::from_std(stream)?;
                Ok(Stream::Unix(stream))
            }
        }
    }
}
