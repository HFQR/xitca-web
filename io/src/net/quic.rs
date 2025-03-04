use core::net::SocketAddr;

use std::io;

use quinn::{Endpoint, Incoming, ServerConfig};

use super::Stream;

pub type QuicConnecting = Incoming;

pub type QuicConfig = ServerConfig;

/// UdpListener is a wrapper type of [`Endpoint`].
#[derive(Debug)]
pub struct QuicListener {
    endpoint: Endpoint,
}

impl QuicListener {
    pub fn new(endpoint: Endpoint) -> Self {
        Self { endpoint }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl QuicListener {
    /// Accept [`UdpStream`].
    pub async fn accept(&self) -> io::Result<QuicStream> {
        let connecting = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "quinn endpoint is closed"))?;
        Ok(QuicStream { connecting })
    }
}

/// Builder type for [QuicListener]
///
/// Unlike other OS provided listener types(Tcp .etc), the construction of [QuicListener]
/// will be interacting with async runtime so it's desirable to delay it until it enters
/// the context of async runtime. Builder type exists for this purpose.
pub struct QuicListenerBuilder {
    addr: SocketAddr,
    config: ServerConfig,
    /// An artificial backlog capacity reinforced by bounded channel.
    /// The channel is tasked with distribute [UdpStream] and can cache stream up most to
    /// the number equal to backlog.
    backlog: u32,
}

impl QuicListenerBuilder {
    pub const fn new(addr: SocketAddr, config: ServerConfig) -> Self {
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

    pub fn build(self) -> io::Result<QuicListener> {
        let Self { config, addr, .. } = self;
        Endpoint::server(config, addr).map(|endpoint| QuicListener { endpoint })
    }
}

/// Wrapper type for [`Connecting`].
///
/// Naming is to keep consistent with `TcpStream` / `UnixStream`.
pub struct QuicStream {
    connecting: QuicConnecting,
}

impl QuicStream {
    /// Expose [`Connecting`] type that can be polled or awaited.
    ///
    /// # Examples:
    ///
    /// ```rust
    /// # use xitca_io::net::QuicStream;
    /// async fn handle(stream: QuicStream) {
    ///     use quinn::Connection;
    ///     let new_conn: Connection = stream.connecting().await.unwrap();
    /// }
    /// ```
    pub fn connecting(self) -> QuicConnecting {
        self.connecting
    }

    /// Get remote [`SocketAddr`] self connected to.
    pub fn peer_addr(&self) -> SocketAddr {
        self.connecting.remote_address()
    }
}

impl TryFrom<Stream> for QuicStream {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        <(QuicStream, SocketAddr)>::try_from(stream).map(|(udp, _)| udp)
    }
}

impl TryFrom<Stream> for (QuicStream, SocketAddr) {
    type Error = io::Error;

    fn try_from(stream: Stream) -> Result<Self, Self::Error> {
        match stream {
            Stream::Udp(udp, addr) => Ok((udp, addr)),
            _ => unreachable!("Can not be casted to UdpStream"),
        }
    }
}
