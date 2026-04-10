use core::{marker::PhantomData, time::Duration};

use crate::{
    body::{Body, BodyError, BodyExt, BoxBody, Data, RequestBody, Trailers, downcast_body},
    bytes::Bytes,
    client::Client,
    error::Error,
    http::{
        self, Extensions, Method, Version, const_header_value,
        header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, TRAILER},
    },
    response::Response,
    service::ServiceRequest,
};

/// builder type for [http::Request] with extended functionalities.
pub struct RequestBuilder<'a, M = marker::Http> {
    pub(crate) req: http::Request<RequestBody>,
    pub(crate) err: Vec<Error>,
    client: &'a Client,
    timeout: Duration,
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
        self.bytes(text)
    }

    #[cfg(feature = "json")]
    /// Use json object as request body.
    pub fn json(mut self, body: impl serde::ser::Serialize) -> Self {
        match serde_json::to_vec(&body) {
            Ok(body) => {
                self.headers_mut().insert(CONTENT_TYPE, const_header_value::JSON);
                self.bytes(body)
            }
            Err(e) => {
                self.push_error(e.into());
                self
            }
        }
    }

    #[cfg(feature = "multipart")]
    pub fn multipart(mut self, parts: Vec<crate::multipart::Part>) -> Self {
        let form = crate::multipart::Form::new(parts);
        self.headers_mut().insert(CONTENT_TYPE, form.content_type());
        self.method(Method::POST)
            .body(xitca_http::body::StreamDataBody::new(form))
    }

    /// Use pre allocated bytes as request body.
    ///
    /// Input type must implement [From] trait with [Bytes].
    pub fn bytes(mut self, body: impl Into<Bytes>) -> Self {
        let bytes = body.into();
        let val = HeaderValue::from(bytes.len());
        self.headers_mut().insert(CONTENT_LENGTH, val);
        *self.req.body_mut() = RequestBody::bytes(bytes);
        self
    }

    /// Use streaming type as request body.
    #[inline]
    pub fn body<B>(self, body: B) -> Self
    where
        B: Body + Send + 'static,
        B::Data: Into<Bytes>,
        B::Error: Into<BodyError>,
    {
        self.map_body(body)
    }

    /// Append HTTP trailers to the request body.
    ///
    /// The current body is wrapped with [`WithTrailers`] so that trailer frames are
    /// sent after all data frames. The `TRAILER` header is automatically populated
    /// with the trailer field names (required for HTTP/1.1 chunked transfer).
    ///
    /// This method can be called after setting the body via [`body`](Self::body),
    /// [`text`](Self::text), [`json`](Self::json), or [`stream`](Self::stream).
    pub fn trailers(mut self, trailers: HeaderMap) -> Self {
        // Set the TRAILER header with comma-separated field names for h1 compat.
        let names = trailers.keys().map(HeaderName::as_str).collect::<Vec<_>>().join(", ");
        if let Ok(val) = HeaderValue::from_str(&names) {
            self.headers_mut().insert(TRAILER, val);
        }

        // Wrap the current body with WithTrailers.
        self.req = self
            .req
            .map(|body| RequestBody::body(BoxBody::new(Data::new(body).chain(Trailers::new(trailers)))));
        self
    }

    /// Finish request builder and send it to server.
    pub async fn send(self) -> Result<Response, Error> {
        self._send().await
    }
}

impl<'a, M> RequestBuilder<'a, M> {
    pub(crate) fn new<B>(req: http::Request<B>, client: &'a Client) -> Self
    where
        B: Body + Send + 'static,
        B::Data: Into<Bytes>,
        B::Error: Into<BodyError>,
    {
        Self {
            req: req.map(downcast_body),
            err: Vec::new(),
            client,
            timeout: client.timeout_config.request_timeout,
            _marker: PhantomData,
        }
    }

    pub(crate) fn mutate_marker<M2>(self) -> RequestBuilder<'a, M2> {
        RequestBuilder {
            req: self.req,
            err: self.err,
            client: self.client,
            timeout: self.timeout,
            _marker: PhantomData,
        }
    }

    // send request to server
    pub(crate) async fn _send(self) -> Result<Response, Error> {
        let Self {
            mut req,
            err,
            client,
            timeout,
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
                timeout,
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
    /// # Default
    /// When this method is not called the version is chosen from the request URI scheme:
    /// - `https://` / `wss://`: the highest version the enabled crate features allow.
    ///   The final wire protocol is picked by TLS ALPN (or QUIC for HTTP/3), so the server
    ///   can negotiate down to HTTP/1.1 transparently.
    /// - `http://` / `ws://` / `unix://`: HTTP/1.1. HTTP/2 over plaintext is opt-in (see
    ///   below). HTTP/3 is never chosen for plaintext since it requires QUIC.
    ///
    /// # HTTP/2 over plaintext (h2c)
    /// Calling `.version(Version::HTTP_2)` on a plaintext request (e.g. an `http://` URI)
    /// opts into HTTP/2 prior-knowledge: the client writes the HTTP/2 connection preface
    /// directly on the TCP socket. If the origin does not speak HTTP/2 the handshake fails
    /// and the request is transparently retried over HTTP/1.1 on a fresh connection — one
    /// extra TCP round trip is spent on the failed probe. Successful h2c connections are
    /// cached in the shared pool and multiplexed across subsequent requests, so the cost
    /// is paid at most once per pool miss.
    ///
    /// This is also the path used by the gRPC helpers on plaintext origins.
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

    fn map_body<B>(mut self, b: B) -> RequestBuilder<'a, M>
    where
        B: Body + Send + 'static,
        B::Data: Into<Bytes>,
        B::Error: Into<BodyError>,
    {
        self.req = self.req.map(|_| downcast_body(b));
        self
    }
}
