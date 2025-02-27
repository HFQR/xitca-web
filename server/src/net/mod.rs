use std::io;

#[cfg(feature = "quic")]
use xitca_io::net::QuicListenerBuilder;
#[cfg(unix)]
use xitca_io::net::UnixListener;
use xitca_io::net::{ListenObj, Listener, TcpListener};

use tracing::info;

/// Helper trait for converting listener types and register them to xitca-server
/// By delay the conversion and make the process happen in server thread(s) it avoid possible panic due to runtime locality.
pub trait IntoListener: Send {
    fn into_listener(self) -> io::Result<ListenObj>;
}

impl IntoListener for std::net::TcpListener {
    fn into_listener(self) -> io::Result<ListenObj> {
        self.set_nonblocking(true)?;
        let listener = TcpListener::from_std(self)?;
        info!("Started Tcp listening on: {:?}", listener.local_addr().ok());
        Ok(Box::new(Listener::Tcp(listener)))
    }
}

#[cfg(unix)]
impl IntoListener for std::os::unix::net::UnixListener {
    fn into_listener(self) -> io::Result<ListenObj> {
        self.set_nonblocking(true)?;
        let listener = UnixListener::from_std(self)?;
        info!("Started Unix listening on: {:?}", listener.local_addr().ok());
        Ok(Box::new(Listener::Unix(listener)))
    }
}

#[cfg(feature = "quic")]
impl IntoListener for QuicListenerBuilder {
    fn into_listener(self) -> io::Result<ListenObj> {
        let udp = self.build()?;
        info!("Started Udp listening on: {:?}", udp.endpoint().local_addr().ok());
        Ok(Box::new(Listener::Udp(udp)))
    }
}
