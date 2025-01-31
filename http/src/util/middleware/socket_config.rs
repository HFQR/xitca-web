use core::{net::SocketAddr, time::Duration};

use std::io;

use socket2::{SockRef, TcpKeepalive};

use tracing::warn;
use xitca_io::net::{Stream as ServerStream, TcpStream};
use xitca_service::{Service, ready::ReadyService};

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

impl<S, E> Service<Result<S, E>> for SocketConfig {
    type Response = SocketConfigService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| SocketConfigService {
            config: self.clone(),
            service,
        })
    }
}

impl<S> ReadyService for SocketConfigService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.service.ready().await
    }
}

impl<S> Service<(TcpStream, SocketAddr)> for SocketConfigService<S>
where
    S: Service<(TcpStream, SocketAddr)>,
{
    type Response = S::Response;
    type Error = S::Error;

    async fn call(&self, (stream, addr): (TcpStream, SocketAddr)) -> Result<Self::Response, Self::Error> {
        self.try_apply_config(&stream);
        self.service.call((stream, addr)).await
    }
}

#[cfg(unix)]
impl<S> Service<(UnixStream, SocketAddr)> for SocketConfigService<S>
where
    S: Service<(UnixStream, SocketAddr)>,
{
    type Response = S::Response;
    type Error = S::Error;

    async fn call(&self, (stream, addr): (UnixStream, SocketAddr)) -> Result<Self::Response, Self::Error> {
        self.try_apply_config(&stream);
        self.service.call((stream, addr)).await
    }
}

impl<S> Service<ServerStream> for SocketConfigService<S>
where
    S: Service<ServerStream>,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    async fn call(&self, stream: ServerStream) -> Result<Self::Response, Self::Error> {
        #[cfg_attr(windows, allow(irrefutable_let_patterns))]
        if let ServerStream::Tcp(ref tcp, _) = stream {
            self.try_apply_config(tcp)
        };

        #[cfg(unix)]
        if let ServerStream::Unix(ref unix, _) = stream {
            self.try_apply_config(unix)
        };

        self.service.call(stream).await
    }
}

pub struct SocketConfigService<S> {
    config: SocketConfig,
    service: S,
}

impl<S> SocketConfigService<S> {
    fn apply_config<'s>(&self, stream: impl Into<SockRef<'s>>) -> io::Result<()> {
        let stream_ref = stream.into();

        stream_ref.set_nodelay(self.config.nodelay)?;

        if let Some(ka) = self.config.ka.as_ref() {
            stream_ref.set_tcp_keepalive(ka)?;
        }

        Ok(())
    }

    fn try_apply_config<'s>(&self, stream: impl Into<SockRef<'s>>) {
        if let Err(e) = self.apply_config(stream) {
            warn!(target: "SocketConfig", "Failed to apply configuration to SocketConfig. {:?}", e);
        };
    }
}
