use std::{convert::Infallible, future::Future, io, net::SocketAddr, time::Duration};

use socket2::{SockRef, TcpKeepalive};

use tracing::warn;
use xitca_io::net::{Stream as ServerStream, TcpStream};
use xitca_service::{ready::ReadyService, Service};

/// A middleware for socket options config of [TcpStream].
#[derive(Clone, Debug)]
pub struct TcpConfig {
    ka: Option<TcpKeepalive>,
    nodelay: bool,
}

impl Default for TcpConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl TcpConfig {
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

impl<S> Service<S> for TcpConfig {
    type Response = TcpConfigService<S>;
    type Error = Infallible;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where S: 'f;

    fn call<'s>(&'s self, service: S) -> Self::Future<'s>
    where
        S: 's,
    {
        let config = self.clone();
        async { Ok(TcpConfigService { config, service }) }
    }
}

impl<S> ReadyService for TcpConfigService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;
    type ReadyFuture<'f> = S::ReadyFuture<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}

impl<S> Service<(TcpStream, SocketAddr)> for TcpConfigService<S>
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

impl<S> Service<ServerStream> for TcpConfigService<S>
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
        // Windows OS specific lint.
        #[allow(irrefutable_let_patterns)]
        if let ServerStream::Tcp(ref tcp, _) = stream {
            self.try_apply_config(tcp);
        }

        self.service.call(stream)
    }
}

pub struct TcpConfigService<S> {
    config: TcpConfig,
    service: S,
}

impl<S> TcpConfigService<S> {
    fn apply_config(&self, stream: &TcpStream) -> io::Result<()> {
        let stream_ref = SockRef::from(stream);
        stream_ref.set_nodelay(self.config.nodelay)?;

        if let Some(ka) = self.config.ka.as_ref() {
            stream_ref.set_tcp_keepalive(ka)?;
        }

        Ok(())
    }

    fn try_apply_config(&self, stream: &TcpStream) {
        if let Err(e) = self.apply_config(stream) {
            warn!(target: "TcpConfig", "Failed to apply configuration to TcpStream. {:?}", e);
        };
    }
}
