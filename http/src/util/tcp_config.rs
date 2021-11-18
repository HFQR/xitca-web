use std::{future::Future, io, time::Duration};

use socket2::{SockRef, TcpKeepalive};
use tracing::warn;
use xitca_io::net::{Stream as ServerStream, TcpStream};
use xitca_service::{Service, Transform};

#[derive(Clone, Debug)]
pub struct TcpConfig {
    ka: Option<TcpKeepalive>,
    no_delay: bool,
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
            no_delay: false,
        }
    }

    /// See [socket2::Socket::set]
    pub fn set_no_delay(mut self, value: bool) -> Self {
        self.no_delay = value;
        self
    }

    pub fn keep_alive_with_time(mut self, time: Duration) -> Self {
        self.ka = Some(self.ka.unwrap_or_else(TcpKeepalive::new).with_time(time));
        self
    }

    pub fn keep_alive_with_interval(mut self, time: Duration) -> Self {
        self.ka = Some(self.ka.unwrap_or_else(TcpKeepalive::new).with_interval(time));
        self
    }

    pub fn keep_alive_with_retries(mut self, retries: u32) -> Self {
        self.ka = Some(self.ka.unwrap_or_else(TcpKeepalive::new).with_retries(retries));
        self
    }
}

impl<S> Transform<S, TcpStream> for TcpConfig
where
    S: Service<TcpStream>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Transform = TcpConfigMiddleware<S>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let config = self.clone();
        async move { Ok(TcpConfigMiddleware { config, service }) }
    }
}

impl<S> Service<TcpStream> for TcpConfigMiddleware<S>
where
    S: Service<TcpStream>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Ready<'f>
    where
        S: 'f,
    = S::Ready<'f>;
    type Future<'f>
    where
        S: 'f,
    = S::Future<'f>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        self.service.ready()
    }

    #[inline]
    fn call(&self, req: TcpStream) -> Self::Future<'_> {
        self.try_apply_config(&req);
        self.service.call(req)
    }
}

impl<S> Transform<S, ServerStream> for TcpConfig
where
    S: Service<ServerStream>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Transform = TcpConfigMiddleware<S>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let config = self.clone();
        async move { Ok(TcpConfigMiddleware { config, service }) }
    }
}

impl<S> Service<ServerStream> for TcpConfigMiddleware<S>
where
    S: Service<ServerStream>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Ready<'f>
    where
        S: 'f,
    = S::Ready<'f>;
    type Future<'f>
    where
        S: 'f,
    = S::Future<'f>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        self.service.ready()
    }

    #[inline]
    fn call(&self, req: ServerStream) -> Self::Future<'_> {
        match req {
            ServerStream::Tcp(ref tcp) => {
                self.try_apply_config(tcp);
                self.service.call(req)
            }
            req => self.service.call(req),
        }
    }
}

pub struct TcpConfigMiddleware<S> {
    config: TcpConfig,
    service: S,
}

impl<S> TcpConfigMiddleware<S> {
    fn apply_config(&self, stream: &TcpStream) -> io::Result<()> {
        let stream_ref = SockRef::from(stream);
        stream_ref.set_nodelay(self.config.no_delay)?;

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
