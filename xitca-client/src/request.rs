use std::time::Duration;

use futures_core::Stream;
use xitca_http::http::{self, header::HeaderMap, Method, Version};

use crate::{client::Client, connect::Connect, connection::Connection, error::Error, uri::Uri};

/// crate level HTTP request type.
pub struct Request<'a, B> {
    /// HTTP request type from [http] crate.
    req: http::Request<B>,
    /// Referece to Client instance.
    client: &'a Client,
    /// Request level timeout setting. When Some(Duration) would override
    /// timeout configuration from Client.
    timeout: Option<Duration>,
}

impl<'a, B> Request<'a, B> {
    pub(crate) fn new(req: http::Request<B>, client: &'a Client) -> Self {
        Self {
            req,
            client,
            timeout: None,
        }
    }

    /// Returns request's headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        self.req.headers()
    }

    /// Returns request's mutable headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.req.headers_mut()
    }

    /// Set HTTP method of this request.
    pub fn method(mut self, method: Method) -> Self {
        *self.req.method_mut() = method;
        self
    }

    #[doc(hidden)]
    /// Set HTTP version of this request.
    ///
    /// By default request's HTTP version depends on network stream
    pub fn version(mut self, version: Version) -> Self {
        *self.req.version_mut() = version;
        self
    }

    /// Set timeout of this request.
    ///
    /// The vaule passed would override global [TimeoutConfig](crate::timeout::TimeoutConfig).
    #[inline]
    pub fn timeout(self, dur: Duration) -> Self {
        self._timeout(Some(dur))
    }

    fn _timeout(mut self, dur: Option<Duration>) -> Self {
        self.timeout = dur;
        self
    }

    pub fn map_body<F, B1>(self, f: F) -> Request<'a, B1>
    where
        F: FnOnce(B) -> B1,
    {
        let Self { req, client, timeout } = self;
        let (parts, body_old) = req.into_parts();

        let body = f(body_old);
        let req = http::Request::from_parts(parts, body);

        Request::new(req, client)._timeout(timeout)
    }

    pub fn replace_body<B1>(self, body: B1) -> (Request<'a, B1>, B) {
        let Self { req, client, timeout } = self;
        let (parts, body_old) = req.into_parts();

        let req = http::Request::from_parts(parts, body);

        let req = Request::new(req, client)._timeout(timeout);

        (req, body_old)
    }

    /// Send the request and return response asynchronously.
    pub async fn send<T, E>(self) -> Result<http::Response<()>, Error>
    where
        B: Stream<Item = Result<T, E>>,
        T: AsRef<[u8]>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let Self { req, client, timeout } = self;

        let uri = Uri::try_parse(req.uri())?;

        // Try to grab a connection from pool.
        let mut conn = client.pool.acquire(&uri).await?;

        let conn_is_none = conn.is_none();

        // setup timer according to outcome and timeout configs.
        let dur = match (conn_is_none, timeout) {
            (true, _) => client.timeout_config.resolve_timeout,
            // request's own timeout should override the global one from TimeoutConfig.
            (false, Some(timeout)) => timeout,
            (false, None) => client.timeout_config.request_timeout,
        };

        let timer = tokio::time::sleep(dur);
        tokio::pin!(timer);

        // Nothing in the pool. construct new connection and add it to Conn.
        if conn_is_none {
            let mut connect = Connect::new(uri);

            let c = client.make_connection(&mut connect, timer.as_mut()).await?;
            conn.add_conn(c);
        }

        match conn.inner_ref() {
            Connection::Tcp(stream) => crate::h1::proto::run(stream, req).await?,
            Connection::Tls(stream) => crate::h1::proto::run(stream, req).await?,
            _ => todo!(),
        }

        Ok(http::Response::new(()))
    }
}
