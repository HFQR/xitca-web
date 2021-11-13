use std::{io, net};

use xitca_io::net::{Listener, Stream, TcpListener, TcpStream};
#[cfg(feature = "http3")]
use xitca_io::net::{UdpListenerBuilder, UdpStream};
#[cfg(unix)]
use xitca_io::net::{UnixListener, UnixStream};

use tracing::info;

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

#[cfg(feature = "http3")]
impl FromStream for UdpStream {
    fn from_stream(stream: Stream) -> Self {
        match stream {
            Stream::Udp(udp) => udp,
            _ => unreachable!("Can not be casted to UdpStream"),
        }
    }
}

/// Helper trait for convert listener types to tokio types.
/// This is to delay the conversion and make it happen in server thread(s).
/// Otherwise it could panic.
pub(crate) trait AsListener: Send {
    fn as_listener(&mut self) -> io::Result<Listener>;
}

impl AsListener for Option<net::TcpListener> {
    fn as_listener(&mut self) -> io::Result<Listener> {
        let this = self.take().unwrap();
        this.set_nonblocking(true)?;

        let tcp = TcpListener::from_std(this)?;

        info!("Started Tcp listening on: {:?}", tcp.local_addr().ok());

        Ok(Listener::Tcp(tcp))
    }
}

#[cfg(unix)]
impl AsListener for Option<std::os::unix::net::UnixListener> {
    fn as_listener(&mut self) -> io::Result<Listener> {
        let this = self.take().unwrap();
        this.set_nonblocking(true)?;

        let unix = UnixListener::from_std(this)?;

        info!("Started Unix listening on: {:?}", unix.local_addr().ok());

        Ok(Listener::Unix(unix))
    }
}

#[cfg(feature = "http3")]
impl AsListener for Option<UdpListenerBuilder> {
    fn as_listener(&mut self) -> io::Result<Listener> {
        let udp = self.take().unwrap().build()?;

        info!("Started Udp listening on: {:?}", udp.endpoint().local_addr().ok());

        Ok(Listener::Udp(udp))
    }
}
