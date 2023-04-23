use core::{convert::Infallible, future::Future, time::Duration};

use std::{io, net::SocketAddr, os::fd::AsFd};

use socket2::{SockRef, TcpKeepalive};

use tracing::warn;
use xitca_io::net::{Stream as ServerStream, TcpStream};
use xitca_service::{ready::ReadyService, Service};

#[cfg(unix)]
use xitca_io::net::UnixStream;

/// A middleware for socket options config of `TcpStream` and `UnixStream`.
#[derive(Clone, Debug)]
pub struct SocketConfig {
    ka: Option<TcpKeepalive>,
    nodelay: bool,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl SocketConfig {
    pub const fn new() -> Self {
        Self {
            ka: None,
            nodelay: false,
        }
    }

    /// For more information about this option, see [`set_nodelay`].
    ///
    /// [`set_nodelay`]: socket2::Socket::set_nodelay
    pub fn set_nodelay(mut self, value: bool) -> Self {
        self.nodelay = value;
        self
    }

    /// For more information about this option, see [`with_time`].
    ///
    /// [`with_time`]: TcpKeepalive::with_time
    pub fn keep_alive_with_time(mut self, time: Duration) -> Self {
        self.ka = Some(self.ka.unwrap_or_else(TcpKeepalive::new).with_time(time));
        self
    }

    /// For more information about this option, see [`with_interval`].
    ///
    /// [`with_interval`]: TcpKeepalive::with_interval
    pub fn keep_alive_with_interval(mut self, time: Duration) -> Self {
        self.ka = Some(self.ka.unwrap_or_else(TcpKeepalive::new).with_interval(time));
        self
    }

    #[cfg(not(windows))]
    /// For more information about this option, see [`with_retries`].
    ///
    /// [`with_retries`]: TcpKeepalive::with_retries
    pub fn keep_alive_with_retries(mut self, retries: u32) -> Self {
        self.ka = Some(self.ka.unwrap_or_else(TcpKeepalive::new).with_retries(retries));
        self
    }
}

impl<S> Service<S> for SocketConfig {
    type Response = SocketConfigService<S>;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where S: 'f;

    fn call<'s>(&'s self, service: S) -> Self::Future<'s>
    where
        S: 's,
    {
        let config = self.clone();
        async { Ok(SocketConfigService { config, service }) }
    }
}

impl<S> ReadyService for SocketConfigService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type Future<'f> = S::Future<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::Future<'_> {
        self.service.ready()
    }
}

impl<S> Service<(TcpStream, SocketAddr)> for SocketConfigService<S>
where
    S: Service<(TcpStream, SocketAddr)>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = S::Future<'f> where S: 'f;

    #[inline]
    fn call<'s>(&'s self, (stream, addr): (TcpStream, SocketAddr)) -> Self::Future<'s>
    where
        TcpStream: 's,
    {
        self.try_apply_config(&stream);
        self.service.call((stream, addr))
    }
}

#[cfg(unix)]
impl<S> Service<(UnixStream, SocketAddr)> for SocketConfigService<S>
where
    S: Service<(UnixStream, SocketAddr)>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = S::Future<'f> where S: 'f;

    #[inline]
    fn call<'s>(&'s self, (stream, addr): (UnixStream, SocketAddr)) -> Self::Future<'s>
    where
        UnixStream: 's,
    {
        self.try_apply_config(&stream);
        self.service.call((stream, addr))
    }
}

impl<S> Service<ServerStream> for SocketConfigService<S>
where
    S: Service<ServerStream>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = S::Future<'f> where S: 'f;

    #[inline]
    fn call<'s>(&'s self, stream: ServerStream) -> Self::Future<'s>
    where
        ServerStream: 's,
    {
        if let ServerStream::Tcp(ref tcp, _) = stream {
            self.try_apply_config(tcp)
        };

        #[cfg(unix)]
        if let ServerStream::Unix(ref unix, _) = stream {
            self.try_apply_config(unix)
        };

        self.service.call(stream)
    }
}

pub struct SocketConfigService<S> {
    config: SocketConfig,
    service: S,
}

impl<S> SocketConfigService<S> {
    fn apply_config(&self, stream: &impl AsFd) -> io::Result<()> {
        let stream_ref = SockRef::from(stream);

        stream_ref.set_nodelay(self.config.nodelay)?;

        if let Some(ka) = self.config.ka.as_ref() {
            stream_ref.set_tcp_keepalive(ka)?;
        }

        Ok(())
    }

    fn try_apply_config(&self, stream: &impl AsFd) {
        if let Err(e) = self.apply_config(stream) {
            warn!(target: "SocketConfig", "Failed to apply configuration to TcpStream. {:?}", e);
        };
    }
}
