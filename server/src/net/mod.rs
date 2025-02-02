use std::{io, net};

#[cfg(feature = "quic")]
use xitca_io::net::QuicListenerBuilder;
#[cfg(unix)]
use xitca_io::net::UnixListener;
use xitca_io::net::{Listener, TcpListener};

use tracing::info;

/// Helper trait for converting listener types and register them to xitca-server
/// This is to delay the conversion and make the process happen in server thread(s).
/// Otherwise it could panic due runtime locality.
pub trait IntoListener: Send {
    fn into_listener(self) -> io::Result<Listener>;
}

impl IntoListener for TcpListener {
    fn into_listener(self) -> io::Result<Listener> {
        info!("Started Tcp listening on: {:?}", self.local_addr().ok());
        Ok(Listener::Tcp(self))
    }
}

impl IntoListener for net::TcpListener {
    fn into_listener(self) -> io::Result<Listener> {
        self.set_nonblocking(true)?;
        TcpListener::from_std(self)?.into_listener()
    }
}

#[cfg(unix)]
impl IntoListener for UnixListener {
    fn into_listener(self) -> io::Result<Listener> {
        info!("Started Unix listening on: {:?}", self.local_addr().ok());
        Ok(Listener::Unix(self))
    }
}

#[cfg(unix)]
impl IntoListener for std::os::unix::net::UnixListener {
    fn into_listener(self) -> io::Result<Listener> {
        self.set_nonblocking(true)?;
        UnixListener::from_std(self)?.into_listener()
    }
}

#[cfg(feature = "quic")]
impl IntoListener for QuicListenerBuilder {
    fn into_listener(self) -> io::Result<Listener> {
        let udp = self.build()?;
        info!("Started Udp listening on: {:?}", udp.endpoint().local_addr().ok());
        Ok(Listener::Udp(udp))
    }
}
