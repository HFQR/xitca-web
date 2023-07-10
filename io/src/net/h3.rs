use std::{io, net::SocketAddr};

use quinn::{Connecting, Endpoint, ServerConfig};

use super::Stream;

pub type UdpConnecting = Connecting;

pub type H3ServerConfig = ServerConfig;

/// UdpListener is a wrapper type of [`Endpoint`].
#[derive(Debug)]
pub struct UdpListener {
    endpoint: Endpoint,
}

impl UdpListener {
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl UdpListener {
    /// Accept [`UdpStream`].
    pub async fn accept(&self) -> io::Result<UdpStream> {
        let connecting = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "quinn endpoint is closed"))?;
        Ok(UdpStream { connecting })
    }
}

/// Builder type for UdpListener.
pub struct UdpListenerBuilder {
    addr: SocketAddr,
    config: ServerConfig,
    /// An artificial backlog capacity reinforced by bounded channel.
    /// The channel is tasked with distribute [UdpStream] and can cache stream up most to
    /// the number equal to backlog.
    backlog: u32,
}

impl UdpListenerBuilder {
    pub fn new(addr: SocketAddr, config: ServerConfig) -> Self {
        Self {
            addr,
            config,
            backlog: 2048,
        }
    }

    pub fn backlog(mut self, backlog: u32) -> Self {
        self.backlog = backlog;
        self
    }

    pub fn build(self) -> io::Result<UdpListener> {
        let Self { config, addr, .. } = self;
        Endpoint::server(config, addr).map(|endpoint| UdpListener { endpoint })
    }
}

/// Wrapper type for [`Connecting`].
///
/// Naming is to keep consistent with `TcpStream` / `UnixStream`.
pub struct UdpStream {
    connecting: Connecting,
}

impl UdpStream {
    /// Expose [`Connecting`] type that can be polled or awaited.
    ///
    /// # Examples:
    ///
    /// ```rust
    /// # use xitca_io::net::UdpStream;
    /// async fn handle(stream: UdpStream) {
    ///     use quinn::Connection;
    ///     let new_conn: Connection = stream.connecting().await.unwrap();
    /// }
    /// ```
    pub fn connecting(self) -> Connecting {
        self.connecting
    }

    /// Get remote [`SocketAddr`] self connected to.
    pub fn peer_addr(&self) -> SocketAddr {
        self.connecting.remote_address()
    }
}

impl TryFrom<Stream> for UdpStream {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        <(UdpStream, SocketAddr)>::try_from(stream).map(|(udp, _)| udp)
    }
}

impl TryFrom<Stream> for (UdpStream, SocketAddr) {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        match stream {
            Stream::Udp(udp, addr) => Ok((udp, addr)),
            _ => unreachable!("Can not be casted to UdpStream"),
        }
    }
}
