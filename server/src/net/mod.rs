use std::{io, net};

#[cfg(feature = "quic")]
use xitca_io::net::QuicListenerBuilder;
#[cfg(unix)]
use xitca_io::net::UnixListener;
use xitca_io::net::{Listener, TcpListener};

use tracing::info;

/// Helper trait for convert listener types to tokio types.
/// This is to delay the conversion and make it happen in server thread(s).
/// Otherwise it could panic.
pub trait AsListener: Send {
    fn as_listener(&mut self) -> io::Result<Listener>;
}

impl<T> AsListener for Option<T>
where
    T: AsListener,
{
    fn as_listener(&mut self) -> io::Result<Listener> {
        let mut this = self.take().unwrap();

        this.as_listener()
    }
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

#[cfg(feature = "quic")]
impl AsListener for Option<QuicListenerBuilder> {
    fn as_listener(&mut self) -> io::Result<Listener> {
        let udp = self.take().unwrap().build()?;

        info!("Started Udp listening on: {:?}", udp.endpoint().local_addr().ok());

        Ok(Listener::Udp(udp))
    }
}
