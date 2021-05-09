use std::io;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{future::Future, net::SocketAddr};

use async_channel::{Receiver, Recv};
use futures_core::ready;
use h3_quinn::quinn::{
    crypto::{rustls::TlsSession, Session},
    generic::{Connecting, Endpoint, ServerConfig},
};
use tokio::task::JoinHandle;

use super::{AsListener, FromStream, Listener, Stream};

pub type H3ServerConfig = ServerConfig<TlsSession>;

#[derive(Debug)]
pub struct UdpListener<S = TlsSession>
where
    S: Session,
{
    endpoint: Endpoint<S>,
    incoming: Receiver<Connecting<S>>,
    handle: JoinHandle<()>,
}

impl<S: Session> Drop for UdpListener<S> {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl UdpListener {
    pub fn accept(&self) -> Accept<'_> {
        Accept {
            recv: self.incoming.recv(),
        }
    }
}

pub struct UdpListenerBuilder<S: Session = TlsSession> {
    addr: SocketAddr,
    config: ServerConfig<S>,
}

impl AsListener for Option<UdpListenerBuilder> {
    fn as_listener(&mut self) -> io::Result<Listener> {
        let this = self.take().unwrap();
        this.build().map(Listener::Udp)
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

pub struct Accept<'a> {
    recv: Recv<'a, Connecting<TlsSession>>,
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

pub struct UdpStream {
    connecting: Connecting<TlsSession>,
}

impl Deref for UdpStream {
    type Target = Connecting<TlsSession>;

    fn deref(&self) -> &Self::Target {
        &self.connecting
    }
}

impl DerefMut for UdpStream {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connecting
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
