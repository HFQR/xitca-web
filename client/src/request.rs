use core::{marker::PhantomData, time::Duration};

use futures_core::Stream;

use crate::{
    body::{BodyError, BoxBody, Once},
    bytes::Bytes,
    client::Client,
    error::Error,
    http::{
        self, const_header_value,
        header::{HeaderMap, HeaderValue, CONTENT_LENGTH, CONTENT_TYPE},
        Extensions, Method, Version,
    },
    response::Response,
    service::ServiceRequest,
    timeout::PartialTimeoutConfig,
};

/// builder type for [http::Request] with extended functionalities.
pub struct RequestBuilder<'a, M = marker::Http> {
    pub(crate) req: http::Request<BoxBody>,
    err: Vec<Error>,
    timeout_config: PartialTimeoutConfig,
    client: &'a Client,
    _marker: PhantomData<M>,
}

// default marker for normal http request
mod marker {
    pub struct Http;
}

impl RequestBuilder<'_, marker::Http> {
    /// Set HTTP method of this request.
    #[inline]
    pub fn method(mut self, method: Method) -> Self {
        *self.req.method_mut() = method;
        self
    }

    /// Use text(utf-8 encoded) as request body.
    ///
    /// [CONTENT_TYPE] header would be set with value: `text/plain; charset=utf-8`.
    pub fn text<B1>(mut self, text: B1) -> Self
    where
        Bytes: From<B1>,
    {
        self.headers_mut().insert(CONTENT_TYPE, const_header_value::TEXT_UTF8);
        self.body(text)
    }

    #[cfg(feature = "json")]
    /// Use json object as request body.
    pub fn json(mut self, body: impl serde::ser::Serialize) -> Self {
        match serde_json::to_vec(&body) {
            Ok(body) => {
                self.headers_mut().insert(CONTENT_TYPE, const_header_value::JSON);
                self.body(body)
            }
            Err(e) => {
                self.push_error(e.into());
                self
            }
        }
    }

    /// Use pre allocated bytes as request body.
    ///
    /// Input type must implement [From] trait with [Bytes].
    pub fn body<B>(mut self, body: B) -> Self
    where
        Bytes: From<B>,
    {
        let bytes = Bytes::from(body);
        let val = HeaderValue::from(bytes.len());
        self.headers_mut().insert(CONTENT_LENGTH, val);
        self.map_body(Once::new(bytes))
    }

    /// Use streaming type as request body.
    #[inline]
    pub fn stream<B, E>(self, body: B) -> Self
    where
        B: Stream<Item = Result<Bytes, E>> + Send + 'static,
        E: Into<BodyError>,
    {
        self.map_body(body)
    }

    /// Finish request builder and send it to server.
    pub async fn send(self) -> Result<Response, Error> {
        self._send().await
    }
}

impl<'a, M> RequestBuilder<'a, M> {
    pub(crate) fn new<B, E>(req: http::Request<B>, client: &'a Client) -> Self
    where
        B: Stream<Item = Result<Bytes, E>> + Send + 'static,
        E: Into<BodyError>,
    {
        Self {
            req: req.map(BoxBody::new),
            err: Vec::new(),
            client,
            timeout_config: PartialTimeoutConfig::new(),
            _marker: PhantomData,
        }
    }

    pub(crate) fn mutate_marker<M2>(self) -> RequestBuilder<'a, M2> {
        RequestBuilder {
            req: self.req,
            err: self.err,
            client: self.client,
            timeout_config: self.timeout_config,
            _marker: PhantomData,
        }
    }

    // send request to server
    pub(crate) async fn _send(self) -> Result<Response, Error> {
        let Self {
            mut req,
            err,
            client,
            timeout_config,
            ..
        } = self;

        if !err.is_empty() {
            return Err(err.into());
        }

        client
            .service
            .call(ServiceRequest {
                req: &mut req,
                client,
                timeout_config: timeout_config.to_timeout_config(&client.timeout_config),
            })
            .await
    }

    pub(crate) fn push_error(&mut self, e: Error) {
        self.err.push(e);
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

    /// Returns request's [Extensions].
    #[inline]
    pub fn extensions(&self) -> &Extensions {
        self.req.extensions()
    }

    /// Returns request's mutable [Extensions].
    #[inline]
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        self.req.extensions_mut()
    }

    /// Set HTTP version of this request.
    ///
    /// By default request's HTTP version depends on network stream
    ///
    /// # Panic
    /// - when received a version beyond the range crate is able to handle.
    /// ```
    /// // depend on default feature of xitca-client in Cargo.toml
    /// // [dependencies]
    /// // xitca-client = { version = "*" }
    ///
    /// fn config(mut req: xitca_client::RequestBuilder<'_>) {
    ///     // this is ok as by default xitca-client can handle http1 request.
    ///     req = req.version(xitca_client::http::Version::HTTP_09);
    ///     // bellow would cause panic as http2/3 are additive features.
    ///     req = req.version(xitca_client::http::Version::HTTP_2);
    ///     req = req.version(xitca_client::http::Version::HTTP_3);
    /// }
    ///
    /// // enable additive http features and the panic would be gone.
    /// // #[dependencies]
    /// // xitca-client = { version = "*", features = ["http2", "http3"] }
    /// ```
    pub fn version(mut self, version: Version) -> Self {
        crate::builder::version_check(version);
        *self.req.version_mut() = version;
        self
    }

    /// Set timeout for DNS resolve.
    ///
    /// Default to client's [TimeoutConfig::resolve_timeout].
    pub fn set_resolve_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.resolve_timeout = Some(dur);
        self
    }

    /// Set timeout for establishing connection.
    ///
    /// Default to client's [TimeoutConfig::connect_timeout].
    pub fn set_connect_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.connect_timeout = Some(dur);
        self
    }

    /// Set timeout for tls handshake.
    ///
    /// Default to client's [TimeoutConfig::tls_connect_timeout].
    pub fn set_tls_connect_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.tls_connect_timeout = Some(dur);
        self
    }

    /// Set timeout for request.
    ///
    /// Default to client's [TimeoutConfig::request_timeout].
    pub fn set_request_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.request_timeout = Some(dur);
        self
    }

    /// Set timeout for collecting response body.
    ///
    /// Default to client's [TimeoutConfig::response_timeout].
    pub fn set_response_timeout(mut self, dur: Duration) -> Self {
        self.timeout_config.response_timeout = Some(dur);
        self
    }

    fn map_body<B, E>(mut self, b: B) -> RequestBuilder<'a, M>
    where
        B: Stream<Item = Result<Bytes, E>> + Send + 'static,
        E: Into<BodyError>,
    {
        self.req = self.req.map(|_| BoxBody::new(b));
        self
    }
}
