#[cfg(feature = "http3")]
mod h3;
mod io;

#[cfg(feature = "http3")]
pub use self::h3::{H3ServerConfig, UdpConnecting, UdpListener, UdpListenerBuilder, UdpStream};

pub use self::io::AsyncReadWrite;
pub use tokio::net::{TcpListener, TcpSocket, TcpStream};

#[cfg(unix)]
pub use tokio::net::{UnixListener, UnixStream};

use std::net;

use tracing::info;

#[derive(Debug)]
pub(crate) enum Listener {
    Tcp(TcpListener),
    #[cfg(feature = "http3")]
    Udp(UdpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

/// A collection of stream types of different protocol.
pub enum Stream {
    Tcp(TcpStream),
    #[cfg(feature = "http3")]
    Udp(UdpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

pub trait FromStream {
    fn from_stream(stream: Stream) -> Self;
}

impl FromStream for Stream {
    fn from_stream(stream: Stream) -> Self {
        stream
    }
}

impl FromStream for TcpStream {
    fn from_stream(stream: Stream) -> Self {
        match stream {
            Stream::Tcp(tcp) => tcp,
            #[cfg(feature = "http3")]
            Stream::Udp(_) => unreachable!("Can not be casted to TcpStream"),
            #[cfg(unix)]
            Stream::Unix(_) => unreachable!("Can not be casted to TcpStream"),
        }
    }
}

#[cfg(unix)]
impl FromStream for UnixStream {
    fn from_stream(stream: Stream) -> Self {
        match stream {
            Stream::Unix(unix) => unix,
            _ => unreachable!("Can not be casted to UnixStream"),
        }
    }
}

/// Helper trait for convert listener types to tokio types.
/// This is to delay the conversion and make it happen in server thread(s).
/// Otherwise it could panic.
pub(crate) trait AsListener: Send {
    fn as_listener(&mut self) -> std::io::Result<Listener>;
}

impl AsListener for Option<net::TcpListener> {
    fn as_listener(&mut self) -> std::io::Result<Listener> {
        let this = self.take().unwrap();
        this.set_nonblocking(true)?;

        let tcp = TcpListener::from_std(this)?;

        info!("Started Tcp listening on: {:?}", tcp.local_addr().ok());

        Ok(Listener::Tcp(tcp))
    }
}

#[cfg(unix)]
impl AsListener for Option<std::os::unix::net::UnixListener> {
    fn as_listener(&mut self) -> std::io::Result<Listener> {
        let this = self.take().unwrap();
        this.set_nonblocking(true)?;

        let unix = UnixListener::from_std(this)?;

        info!("Started Unix listening on: {:?}", unix.local_addr().ok());

        Ok(Listener::Unix(unix))
    }
}

impl Listener {
    pub(crate) async fn accept(&self) -> std::io::Result<Stream> {
        match *self {
            Self::Tcp(ref tcp) => {
                let (stream, _) = tcp.accept().await?;

                // This two way conversion is to deregister stream from the listener thread's poll
                // and re-register it to current thread's poll.
                let stream = stream.into_std()?;
                let stream = TcpStream::from_std(stream)?;
                Ok(Stream::Tcp(stream))
            }
            #[cfg(feature = "http3")]
            Self::Udp(ref udp) => {
                let stream = udp.accept().await?;
                Ok(Stream::Udp(stream))
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
