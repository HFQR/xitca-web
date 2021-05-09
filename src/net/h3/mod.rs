use std::io;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::{future::Future, net::SocketAddr};

use futures_core::ready;
use h3_quinn::quinn::{
    crypto::{rustls::TlsSession, Session},
    generic::{Connecting, Endpoint, Incoming, ServerConfig},
};

use super::{AsListener, FromStream, Listener, Stream};

pub type H3ServerConfig = ServerConfig<TlsSession>;

#[derive(Debug)]
pub struct UdpListener<S: Session = TlsSession> {
    endpoint: Endpoint<S>,
    incoming: Mutex<Incoming<S>>,
}

impl UdpListener {
    pub fn accept(&self) -> Accept<'_> {
        Accept {
            incoming: &self.incoming,
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

        let (endpoint, incoming) = builder.bind(&addr).unwrap();

        Ok(UdpListener {
            endpoint,
            incoming: Mutex::new(incoming),
        })
    }
}

pub struct Accept<'a> {
    incoming: &'a Mutex<Incoming<TlsSession>>,
}

impl Future for Accept<'_> {
    type Output = io::Result<UdpStream>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut incoming = self.get_mut().incoming.lock().unwrap();

        match ready!(futures_core::Stream::poll_next(
            Pin::new(&mut *incoming),
            cx
        )) {
            Some(connecting) => Poll::Ready(Ok(UdpStream { connecting })),
            None => Poll::Ready(Err(io::Error::new(
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
