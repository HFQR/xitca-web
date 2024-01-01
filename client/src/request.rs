use std::time::Duration;

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
};

/// builder type for [http::Request] with extended functionalities.
pub struct RequestBuilder<'a> {
    /// HTTP request type from [http] crate.
    req: http::Request<BoxBody>,
    /// Reference to Client instance.
    client: &'a Client,
    /// Request level timeout setting. When Some(Duration) would override
    /// timeout configuration from Client.
    timeout: Duration,
}

impl<'a> RequestBuilder<'a> {
    pub(crate) fn new<B, E>(req: http::Request<B>, client: &'a Client) -> Self
    where
        B: Stream<Item = Result<Bytes, E>> + Send + 'static,
        E: Into<BodyError>,
    {
        Self {
            req: req.map(BoxBody::new),
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

    /// Set HTTP method of this request.
    #[inline]
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
    /// The value passed would override global [ClientBuilder::set_request_timeout].
    ///
    /// [ClientBuilder::set_request_timeout]: crate::builder::ClientBuilder::set_request_timeout
    #[inline]
    pub fn timeout(mut self, dur: Duration) -> Self {
        self.timeout = dur;
        self
    }

    /// Use text(utf-8 encoded) as request body.
    ///
    /// [CONTENT_TYPE] header would be set with value: `text/plain; charset=utf-8`.
    pub fn text<B1>(mut self, text: B1) -> RequestBuilder<'a>
    where
        Bytes: From<B1>,
    {
        self.headers_mut().insert(CONTENT_TYPE, const_header_value::TEXT_UTF8);

        self.body(text)
    }

    #[cfg(feature = "json")]
    /// Use json object as request body.
    pub fn json(mut self, body: impl serde::ser::Serialize) -> Result<RequestBuilder<'a>, Error> {
        // TODO: handle serialize error.
        let body = serde_json::to_vec(&body).unwrap();

        self.headers_mut().insert(CONTENT_TYPE, const_header_value::JSON);

        Ok(self.body(body))
    }

    /// Use pre allocated bytes as request body.
    ///
    /// Input type must implement [From] trait with [Bytes].
    pub fn body<B>(mut self, body: B) -> RequestBuilder<'a>
    where
        Bytes: From<B>,
    {
        let bytes = Bytes::from(body);
        self.headers_mut()
            .insert(CONTENT_LENGTH, HeaderValue::from(bytes.len()));
        self.map_body(Once::new(bytes))
    }

    /// Use streaming type as request body.
    #[inline]
    pub fn stream<B, E>(self, body: B) -> RequestBuilder<'a>
    where
        B: Stream<Item = Result<Bytes, E>> + Send + 'static,
        BodyError: From<E>,
    {
        self.map_body(body)
    }

    fn map_body<B, E>(self, b: B) -> RequestBuilder<'a>
    where
        B: Stream<Item = Result<Bytes, E>> + Send + 'static,
        BodyError: From<E>,
    {
        let Self { req, client, timeout } = self;
        let (parts, _) = req.into_parts();
        let body = BoxBody::new(b);
        let req = http::Request::from_parts(parts, body);
        RequestBuilder::new(req, client).timeout(timeout)
    }

    pub async fn send(self) -> Result<Response<'a>, Error> {
        let service = &*self.client.service;
        let Self {
            mut req,
            client,
            timeout,
        } = self;

        service
            .call(ServiceRequest {
                req: &mut req,
                client,
                timeout,
            })
            .await
    }
}
