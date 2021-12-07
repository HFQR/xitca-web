use std::{io, net};

#[cfg(feature = "http3")]
use xitca_io::net::UdpListenerBuilder;
#[cfg(unix)]
use xitca_io::net::UnixListener;
use xitca_io::net::{Listener, TcpListener};

use tracing::info;

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
