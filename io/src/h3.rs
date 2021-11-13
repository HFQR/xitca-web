use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use async_channel::{Receiver, Recv};
use futures_core::ready;
use quinn::{
    crypto::{rustls::TlsSession, Session},
    generic::{Connecting, Endpoint, Incoming, ServerConfig},
    EndpointError,
};

pub type UdpConnecting = Connecting<TlsSession>;

pub type H3ServerConfig = ServerConfig<TlsSession>;

/// UdpListener is a wrapper type of [`Endpoint`](quinn::generic::Endpoint).
#[derive(Debug)]
pub struct UdpListener<S: Session = TlsSession> {
    endpoint: Endpoint<S>,
    /// `async-channel` is used to receive Connecting from [`Incoming`](quinn::generic::Incoming).
    incoming: Receiver<Connecting<S>>,
}

impl<S: Session> UdpListener<S> {
    pub fn endpoint(&self) -> &Endpoint<S> {
        &self.endpoint
    }
}

impl<S: Session> UdpListener<S> {
    /// Accept [`UdpStream`](self::UdpStream).
    pub fn accept(&self) -> Accept<'_, S> {
        Accept {
            recv: self.incoming.recv(),
        }
    }
}

pub struct Accept<'a, S: Session> {
    recv: Recv<'a, Connecting<S>>,
}

impl<S: Session> Future for Accept<'_, S> {
    type Output = io::Result<UdpStream<S>>;

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
pub struct UdpListenerBuilder<S: Session = TlsSession> {
    addr: SocketAddr,
    config: ServerConfig<S>,
}

impl<S> UdpListenerBuilder<S>
where
    S: Session + 'static,
{
    pub fn new(addr: SocketAddr, config: ServerConfig<S>) -> Self {
        Self { addr, config }
    }

    pub fn build(self) -> io::Result<UdpListener<S>> {
        let config = self.config;
        let addr = self.addr;

        let mut builder = Endpoint::builder();
        builder.listen(config);

        let (endpoint, mut incoming) = builder.bind(&addr).map_err(|e| match e {
            EndpointError::Socket(e) => e,
        })?;

        // Use async channel to dispatch Connecting<Session> to worker threads.
        // Incoming can only be held by single task and sharing it between
        // threads would cause hanging.
        let (tx, rx) = async_channel::unbounded();

        // Detach the Incoming<Session> as a spawn task.
        // When Endpoint<Session> dropped incoming will be wakeup and get None to end task.
        tokio::spawn(async move {
            while let Some(conn) = incoming.next().await {
                tx.send(conn).await.unwrap()
            }
        });

        Ok(UdpListener { endpoint, incoming: rx })
    }
}

trait NextTrait<S: Session> {
    fn next(&mut self) -> Next<'_, S>;
}

impl<S: Session> NextTrait<S> for Incoming<S> {
    #[inline(always)]
    fn next(&mut self) -> Next<'_, S> {
        Next { stream: self }
    }
}

struct Next<'a, S: Session> {
    stream: &'a mut Incoming<S>,
}

impl<S: Session> Future for Next<'_, S> {
    type Output = Option<<Incoming<S> as futures_core::Stream>::Item>;

    #[inline(always)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        futures_core::Stream::poll_next(Pin::new(&mut self.get_mut().stream), cx)
    }
}

/// Wrapper type for [`Connecting`](quinn::generic::Connecting).
///
/// Naming is to keep consistent with `TcpStream` / `UnixStream`.
pub struct UdpStream<S: Session = TlsSession> {
    connecting: Connecting<S>,
}

impl<S: Session> UdpStream<S> {
    /// Expose [`Connecting`](quinn::generic::Connecting) type that can be polled or awaited.
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
    pub fn connecting(self) -> Connecting<S> {
        self.connecting
    }

    /// Get remote `SocketAddr` self connected to.
    pub fn peer_addr(&self) -> SocketAddr {
        self.connecting.remote_address()
    }
}
