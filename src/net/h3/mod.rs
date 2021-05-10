use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{future::Future, net::SocketAddr};

use async_channel::{Receiver, Recv};
use futures_core::ready;
use quinn::{
    crypto::{rustls::TlsSession, Session},
    generic::{Connecting, Endpoint, ServerConfig},
};
use tokio::task::JoinHandle;

use super::{AsListener, FromStream, Listener, Stream};

pub type UdpConnecting = Connecting<TlsSession>;

pub type H3ServerConfig = ServerConfig<TlsSession>;

/// UdpListener is a wrapper type of [quinn::generic::Endpoint].
#[derive(Debug)]
pub struct UdpListener<S = TlsSession>
where
    S: Session,
{
    endpoint: Endpoint<S>,
    /// async-channel is used to receive Connecting from [quinn::generic::Incoming]
    incoming: Receiver<Connecting<S>>,
    handle: JoinHandle<()>,
}

impl<S: Session> Drop for UdpListener<S> {
    fn drop(&mut self) {
        // cancel `quinn::generic::Incoming` task on drop.
        self.handle.abort();
    }
}

impl<S: Session> UdpListener<S> {
    /// Accept one [quinn::generic::Connecting].
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

pub struct UdpListenerBuilder<S: Session = TlsSession> {
    addr: SocketAddr,
    config: ServerConfig<S>,
}

impl AsListener for Option<UdpListenerBuilder> {
    fn as_listener(&mut self) -> io::Result<Listener> {
        self.take().unwrap().build().map(Listener::Udp)
    }
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

        let (endpoint, mut incoming) = builder.bind(&addr).unwrap();

        // Use async channel to dispatch Connecting<S> to worker threads.
        // Incoming can only be held by single task and sharing it between
        // threads would cause hanging.
        let (tx, rx) = async_channel::unbounded();
        let handle = tokio::spawn(async move {
            use futures_util::StreamExt;
            while let Some(conn) = incoming.next().await {
                if tx.send(conn).await.is_err() {
                    return;
                }
            }
        });

        Ok(UdpListener {
            endpoint,
            incoming: rx,
            handle,
        })
    }
}

pub struct UdpStream<S: Session = TlsSession> {
    connecting: Connecting<S>,
}

impl<S: Session> UdpStream<S> {
    pub fn connecting(self) -> Connecting<S> {
        self.connecting
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.connecting.remote_address()
    }
}

impl FromStream for UdpStream {
    fn from_stream(stream: Stream) -> Self {
        match stream {
            Stream::Udp(udp) => udp,
            _ => unreachable!("Can not be casted to UdpStream"),
        }
    }
}
