use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use async_channel::{Receiver, Recv};
use futures_core::ready;
use quinn::{Connecting, Endpoint, Incoming, ServerConfig};

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
    pub fn accept(&self) -> Accept<'_> {
        Accept {
            recv: self.incoming.recv(),
        }
    }
}

pub struct Accept<'a> {
    recv: Recv<'a, Connecting>,
}

impl Future for Accept<'_> {
    type Output = io::Result<UdpStream>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match ready!(Pin::new(&mut self.get_mut().recv).poll(cx)) {
            Ok(connecting) => Poll::Ready(Ok(UdpStream { connecting })),
            Err(_) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "quinn endpoint is closed",
            ))),
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
            while let Some(conn) = incoming.next().await {
                tx.send(conn).await.unwrap()
            }
        });

        Ok(UdpListener { endpoint, incoming: rx })
    }
}

trait NextTrait {
    fn next(&mut self) -> Next<'_>;
}

impl NextTrait for Incoming {
    #[inline(always)]
    fn next(&mut self) -> Next<'_> {
        Next { stream: self }
    }
}

struct Next<'a> {
    stream: &'a mut Incoming,
}

impl Future for Next<'_> {
    type Output = Option<<Incoming as futures_core::Stream>::Item>;

    #[inline(always)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        futures_core::Stream::poll_next(Pin::new(&mut self.get_mut().stream), cx)
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
