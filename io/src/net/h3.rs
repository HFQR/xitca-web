use std::{future::poll_fn, io, net::SocketAddr, pin::Pin};

use async_channel::Receiver;
use quinn::{Connecting, Endpoint, ServerConfig};

use super::Stream;

pub type UdpConnecting = Connecting;

pub type H3ServerConfig = ServerConfig;

/// UdpListener is a wrapper type of [`Endpoint`].
#[derive(Debug)]
pub struct UdpListener {
    endpoint: Endpoint,
    /// `async-channel` is used to receive Connecting from [`Incoming`].
    incoming: Receiver<Connecting>,
}

impl UdpListener {
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl UdpListener {
    /// Accept [`UdpStream`].
    pub async fn accept(&self) -> io::Result<UdpStream> {
        match self.incoming.recv().await {
            Ok(connecting) => Ok(UdpStream { connecting }),
            Err(_) => Err(io::Error::new(io::ErrorKind::BrokenPipe, "quinn endpoint is closed")),
        }
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
        let config = self.config;
        let addr = self.addr;
        let backlog = self.backlog;

        let (endpoint, mut incoming) = Endpoint::server(config, addr)?;

        // Use async channel to dispatch Connecting<Session> to worker threads.
        // Incoming can only be held by single task and sharing it between
        // threads would cause hanging.
        let (tx, rx) = async_channel::bounded(backlog as usize);

        // Detach the Incoming> as a spawn task.
        // When Endpoint dropped incoming will be wakeup and get None to end task.
        tokio::spawn(async move {
            use futures_core::stream::Stream;
            let mut incoming = Pin::new(&mut incoming);
            while let Some(conn) = poll_fn(|cx| incoming.as_mut().poll_next(cx)).await {
                tx.send(conn).await.unwrap()
            }
        });

        Ok(UdpListener { endpoint, incoming: rx })
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
    ///     use quinn::NewConnection;
    ///     let new_conn: NewConnection = stream.connecting().await.unwrap();
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

impl From<Stream> for UdpStream {
    fn from(stream: Stream) -> Self {
        match stream {
            Stream::Udp(udp, _) => udp,
            _ => unreachable!("Can not be casted to UdpStream"),
        }
    }
}

impl From<Stream> for (UdpStream, SocketAddr) {
    fn from(stream: Stream) -> Self {
        match stream {
            Stream::Udp(udp, addr) => (udp, addr),
            _ => unreachable!("Can not be casted to UdpStream"),
        }
    }
}
