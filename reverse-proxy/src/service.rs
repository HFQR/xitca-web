use std::cell::{RefCell};
use crate::forwarder::{ForwardError};
use crate::peer_resolver::HttpPeerResolver;
use crate::HttpPeer;
use bytes::Bytes;
use std::collections::HashSet;
use std::ops::{DerefMut};
use std::rc::Rc;
use std::str::FromStr;
use futures::Stream;
use futures_util::StreamExt;
use xitca_client::{Client};
use xitca_http::body::{BoxBody};
use xitca_http::{BodyError};
use xitca_http::http::header::AsHeaderName;
use xitca_http::http::{StatusCode, Version};
use xitca_http::util::service::RouterError;
use xitca_unsafe_collection::fake::{FakeSend, FakeSync};
use xitca_web::error::ErrorStatus;
use xitca_web::http::uri::Scheme;
use xitca_web::http::{header, HeaderMap, HeaderName, Request, Uri, WebResponse};
use xitca_web::service::Service;
use xitca_web::{BodyStream, WebContext};

lazy_static! {
    static ref HOP_HEADERS: HashSet<HeaderName> = {
        let mut hop_headers = HashSet::new();

        hop_headers.insert(header::CONNECTION);
        hop_headers.insert(HeaderName::from_str("proxy-connection").unwrap());
        hop_headers.insert(HeaderName::from_str("keep-alive").unwrap());
        hop_headers.insert(header::PROXY_AUTHENTICATE);
        hop_headers.insert(header::PROXY_AUTHORIZATION);
        hop_headers.insert(header::TE);
        hop_headers.insert(header::TRAILER);
        hop_headers.insert(header::TRANSFER_ENCODING);
        hop_headers.insert(header::UPGRADE);

        hop_headers
    };
}

pub struct ProxyServiceCompat(pub RefCell<ProxyService>);

impl<'r, C, B> Service<WebContext<'r, C, B>> for ProxyServiceCompat
where
    C: Clone + 'static  + Unpin,
    B: BodyStream<Chunk = Bytes, Error = BodyError> + Default + 'static + Unpin,
{
    type Response = WebResponse<BoxBody>;
    type Error = RouterError<ErrorStatus>;

    async fn call(&self, mut ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let (parts, _ext) = ctx.take_request().into_parts();
        let ctx = ctx.state().clone();
        // @TODO this doesn't work when body is empty on HTTP 1.1, as we don't know the size which induce a chunked encoding, and eof is called on that which makes it panics
        // Need to buffer or add a size hint to the body
        let req = Request::from_parts(parts, CompatReqBody::new(BoxBody::default(), ctx));
        let fut = ProxyService::call(&mut *self.0.borrow_mut(), req).await;

        fut
    }
}

pub struct CompatReqBody<B, C> {
    body: FakeSend<B>,
    ctx: FakeSend<FakeSync<C>>,
}

impl<B, C> CompatReqBody<B, C> {
    #[inline]
    pub fn new(body: B, ctx: C) -> Self {
        Self {
            body: FakeSend::new(body),
            ctx: FakeSend::new(FakeSync::new(ctx)),
        }
    }

    /// destruct compat body into owned value of body and state context
    ///
    /// # Panics
    /// - When called from a thread not where B is originally constructed.
    #[inline]
    pub fn into_parts(self) -> (B, C) {
        (self.body.into_inner(), self.ctx.into_inner().into_inner())
    }
}

impl<B, C> Stream for CompatReqBody<B, C> where B: Stream<Item = Result<Bytes, BodyError>> + Unpin, C: Unpin {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        self.get_mut().body.deref_mut().poll_next_unpin(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.body.size_hint()
    }
}

pub struct ProxyService {
    pub(crate) peer_resolver: Rc<HttpPeerResolver>,
    pub(crate) client: Rc<Client>,
}

impl<C, B> Service<Request<CompatReqBody<B, C>>> for ProxyService
where
    C: 'static  + Unpin,
    B: BodyStream<Chunk = Bytes, Error = BodyError> + Default + 'static + Unpin,
{
    type Response = WebResponse<BoxBody>;
    type Error = RouterError<ErrorStatus>;

    async fn call(&self, downstream_request: Request<CompatReqBody<B, C>>) -> Result<Self::Response, Self::Error> {
        let (downstream_request_head, downstream_body) = downstream_request.into_parts();

        let peer = self.peer_resolver.resolve(&downstream_request_head).await.unwrap();

        let mut upstream_request = Request::new(downstream_body);
        *upstream_request.method_mut() = downstream_request_head.method;
        *upstream_request.uri_mut() = match Uri::builder()
            .path_and_query(match downstream_request_head.uri.path_and_query() {
                Some(path_and_query) => path_and_query.as_str(),
                None => downstream_request_head.uri.path(),
            })
            // @TODO only work for http 1.1, need to update lib to be able to separate request host from sni_host
            .authority(peer.sni_host.as_str())
            .scheme(if peer.tls { Scheme::HTTPS } else { Scheme::HTTP })
            .build()
        {
            Err(err) => {
                return Err(RouterError::Service(ForwardError::UriError(err).into_error_status()));
            }
            Ok(url) => url,
        };

        let upstream_request_connection_headers = get_connection_headers(&downstream_request_head.headers);

        for (name, value) in downstream_request_head.headers.iter() {
            if HOP_HEADERS.contains(name) {
                continue;
            }

            if upstream_request_connection_headers.contains(name.as_str()) {
                continue;
            }

            if name == header::HOST {
                continue;
            }

            upstream_request.headers_mut().append(name.clone(), value.clone());
        }

        if contain_value(&downstream_request_head.headers, header::TE, "trailers") {
            upstream_request.headers_mut().insert(header::TE, "trailers".parse().unwrap());
        }

        if let Some(via) = &peer.via {
            if via.add_in_request {
                let version = match downstream_request_head.version {
                    Version::HTTP_09 => Some("0.9"),
                    Version::HTTP_10 => Some("1.0"),
                    Version::HTTP_11 => Some("1.1"),
                    Version::HTTP_2 => Some("2.0"),
                    Version::HTTP_3 => Some("3.0"),
                    _ => None,
                };

                if let Some(version_str) = version {
                    upstream_request
                        .headers_mut()
                        .append(header::VIA, format!("HTTP/{} {}", version_str, via.name).parse().unwrap());
                }
            }
        }

        let current_host = downstream_request_head
            .headers
            .get(header::HOST)
            .map(|v| v.to_str().unwrap_or_default().to_string())
            .or_else(|| downstream_request_head.uri.host().map(|v| v.to_string()))
            .unwrap_or_else(|| "localhost".to_string());

        // @TODO only work for http 1.1, need to update lib to be able to separate request host from sni_host
        upstream_request
            .headers_mut()
            .insert(header::HOST, peer.request_host.parse().unwrap());

        // @TODO Get them from forwaded headers if available
        // let addr = downstream_body.body.socket_addr().ip().to_string();
        let scheme = downstream_request_head.uri.scheme().cloned().unwrap_or(Scheme::HTTP);

        peer.forward_for
            .apply(upstream_request.headers_mut(), "", current_host, scheme);

        // @TODO Handle upgrade request
        *upstream_request.version_mut() = Version::HTTP_2;

        // @TODO Need to set the correct address
        //        upstream_request = upstream_request.address(peer.address);

        // @TODO check bug with http 1.0

        // @TODO Handle invalid certificates
        let client = self.client.as_ref();
        let mut upstream_request_builder = client.request(upstream_request);

        if let Some(timeout) = peer.timeout {
            upstream_request_builder = upstream_request_builder.timeout(timeout);
        }

        let upstream_response = match upstream_request_builder.send().await {
            Ok(res) => res,
            Err(err) => {
                println!("error: {:?}", err);
                // @TODO handle better error
                return Err(RouterError::Service(ErrorStatus::from(StatusCode::from_u16(503).unwrap())));
            }
        };

        let (parts, body) = upstream_response.into_parts();

        // @TODO body into owned is a bad thing, since we lost the capability to having a pool of connections for the client (each request will be a new connection)
        // However without that since we don't have the client in the response we can't have a pool of connections for the client
        let mut response = WebResponse::new(BoxBody::new(body.into_owned()));
        *response.status_mut() = parts.status.clone();

        map_headers(peer, response.headers_mut(), parts.version, parts.status, &parts.headers);

        Ok(response)
    }
}

fn map_headers(peer: Rc<HttpPeer>, downstream_headers: &mut HeaderMap, version: Version, status: StatusCode, upstream_headers: &HeaderMap) {
    let response_connection_headers = get_connection_headers(upstream_headers);

    for (name, value) in upstream_headers {
        // Skip headers only when no switching protocols
        if status != StatusCode::SWITCHING_PROTOCOLS {
            if HOP_HEADERS.contains(name) {
                continue;
            }

            if name == header::CONTENT_LENGTH {
                continue;
            }

            if response_connection_headers.contains(name.as_str()) {
                continue;
            }
        }

        downstream_headers.append(name, value.clone());
    }

    if let Some(via) = &peer.via {
        if via.add_in_response {
            let via_version = match version {
                Version::HTTP_09 => Some("0.9"),
                Version::HTTP_10 => Some("1.0"),
                Version::HTTP_11 => Some("1.1"),
                Version::HTTP_2 => Some("2.0"),
                Version::HTTP_3 => Some("3.0"),
                _ => None,
            };

            if let Some(via_version_str) = via_version {
                downstream_headers.append(header::VIA, format!("HTTP/{} {}", via_version_str, via.name).parse().unwrap());
            }
        }
    }
}

fn get_connection_headers(header_map: &HeaderMap) -> HashSet<String> {
    let mut connection_headers = HashSet::new();

    for conn_value in header_map.get_all("connection") {
        match conn_value.to_str() {
            Err(_) => (),
            Ok(conn_value_str) => {
                for value in conn_value_str.split(',') {
                    match HeaderName::from_str(value.trim()) {
                        Err(_) => (),
                        Ok(header_name) => {
                            connection_headers.insert(header_name.as_str().to_lowercase());
                        }
                    }
                }
            }
        }
    }

    connection_headers
}

fn contain_value(map: &HeaderMap, key: impl AsHeaderName, value: &str) -> bool {
    for val in map.get_all(key) {
        match val.to_str() {
            Err(_) => (),
            Ok(vs) => {
                if value.to_lowercase() == vs.to_lowercase() {
                    return true;
                }
            }
        }
    }

    false
}