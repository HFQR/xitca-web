use std::time::Duration;

use futures_core::Stream;
use tokio::time::Instant;
use xitca_http::{
    bytes::Bytes,
    error::BodyError,
    http::{self, header::HeaderMap, Method, Version},
};

use crate::{
    body::{RequestBody, ResponseBody},
    client::Client,
    connect::Connect,
    connection::Connection,
    error::{Error, TimeoutError},
    response::DefaultResponse,
    timeout::Timeout,
    uri::Uri,
};

/// crate level HTTP request type.
pub struct Request<'a, B> {
    /// HTTP request type from [http] crate.
    req: http::Request<RequestBody<B>>,
    /// Referece to Client instance.
    client: &'a Client,
    /// Request level timeout setting. When Some(Duration) would override
    /// timeout configuration from Client.
    timeout: Duration,
}

impl<'a, B> Request<'a, B> {
    pub(crate) fn new(req: http::Request<RequestBody<B>>, client: &'a Client) -> Self {
        Self {
            req,
            client,
            timeout: client.timeout_config.request_timeout,
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
    /// The value passed would override global [TimeoutConfig].
    #[inline]
    pub fn timeout(mut self, dur: Duration) -> Self {
        self.timeout = dur;
        self
    }

    pub fn map_body<F, B1>(self, f: F) -> Request<'a, B1>
    where
        F: FnOnce(RequestBody<B>) -> RequestBody<B1>,
    {
        let Self { req, client, timeout } = self;
        let (parts, body_old) = req.into_parts();

        let body = f(body_old);
        let req = http::Request::from_parts(parts, body);

        Request::new(req, client).timeout(timeout)
    }

    /// Send the request and return response asynchronously.
    pub async fn send<E>(self) -> Result<DefaultResponse<'a>, Error>
    where
        B: Stream<Item = Result<Bytes, E>>,
        BodyError: From<E>,
    {
        let Self { req, client, timeout } = self;

        let uri = Uri::try_parse(req.uri())?;

        // Try to grab a connection from pool.
        let mut conn = client.pool.acquire(&uri).await?;

        let conn_is_none = conn.is_none();

        // setup timer according to outcome and timeout configs.
        let dur = if conn_is_none {
            client.timeout_config.resolve_timeout
        } else {
            timeout
        };

        // heap allocate timer so it can be moved to Response type afterwards
        let mut timer = Box::pin(tokio::time::sleep(dur));

        // Nothing in the pool. construct new connection and add it to Conn.
        if conn_is_none {
            let mut connect = Connect::new(uri);

            let c = client.make_connection(&mut connect, &mut timer).await?;
            conn.add(c);
        }

        let date = client.date_service.handle();

        timer
            .as_mut()
            .reset(Instant::now() + client.timeout_config.request_timeout);

        match &mut *conn {
            Connection::Tcp(stream) => {
                match crate::h1::proto::send(stream, date, req)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| TimeoutError::Request)
                {
                    Ok(Ok((res, buf, decoder))) => {
                        let body = crate::h1::body::ResponseBody::new(conn, buf, decoder);
                        let res = res.map(|_| ResponseBody::H1(body));
                        let timeout = client.timeout_config.response_timeout;
                        Ok(DefaultResponse::new(res, timer, timeout))
                    }
                    Ok(Err(e)) => {
                        conn.destroy();
                        Err(e.into())
                    }
                    Err(e) => {
                        conn.destroy();
                        Err(e.into())
                    }
                }
            }
            Connection::Tls(stream) => {
                match crate::h1::proto::send(stream, date, req)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| TimeoutError::Request)
                {
                    Ok(Ok((res, buf, decoder))) => {
                        let body = crate::h1::body::ResponseBody::new(conn, buf, decoder);
                        let res = res.map(|_| ResponseBody::H1(body));
                        let timeout = client.timeout_config.response_timeout;
                        Ok(DefaultResponse::new(res, timer, timeout))
                    }
                    Ok(Err(e)) => {
                        conn.destroy();
                        Err(e.into())
                    }
                    Err(e) => {
                        conn.destroy();
                        Err(e.into())
                    }
                }
            }
            Connection::Unix(stream) => {
                match crate::h1::proto::send(stream, date, req)
                    .timeout(timer.as_mut())
                    .await
                    .map_err(|_| TimeoutError::Request)
                {
                    Ok(Ok((res, buf, decoder))) => {
                        let body = crate::h1::body::ResponseBody::new(conn, buf, decoder);
                        let res = res.map(|_| ResponseBody::H1(body));
                        let timeout = client.timeout_config.response_timeout;
                        Ok(DefaultResponse::new(res, timer, timeout))
                    }
                    Ok(Err(e)) => {
                        conn.destroy();
                        Err(e.into())
                    }
                    Err(e) => {
                        conn.destroy();
                        Err(e.into())
                    }
                }
            }
            #[cfg(feature = "http2")]
            Connection::H2(_) => todo!(),
        }
    }
}
