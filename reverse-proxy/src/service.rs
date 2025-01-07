use crate::forwarder::ForwardError;
use crate::peer_resolver::HttpPeerResolver;
use crate::HttpPeer;
use bytes::Bytes;
use futures::Stream;
use std::collections::HashSet;
use std::pin::Pin;
use std::rc::Rc;
use std::str::FromStr;
use std::task::{Context, Poll};
use tokio::io::{copy, AsyncRead, ReadHalf};
use xitca_client::error::Error;
use xitca_client::{Client, HttpTunnel};
use xitca_http::body::BoxBody;
use xitca_http::http::header::AsHeaderName;
use xitca_http::http::{StatusCode, Version};
use xitca_http::util::service::RouterError;
use xitca_http::BodyError;
use xitca_io::io::PollIoAdapter;
use xitca_web::error::ErrorStatus;
use xitca_web::http::uri::Scheme;
use xitca_web::http::{header, HeaderMap, HeaderName, HeaderValue, Request, Uri, WebResponse};
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

pub struct ProxyService {
    pub(crate) peer_resolver: Rc<HttpPeerResolver>,
    pub(crate) client: Rc<Client>,
}

impl ProxyService {
    async fn upgrade(
        &self,
        upstream_request: Request<FakeSend<BoxBody>>,
    ) -> Result<WebResponse<BoxBody>, RouterError<ErrorStatus>> {
        let (upstream_request_head, mut downstream_stream_read) = upstream_request.into_parts();
        let mut upgrade_request = Request::from_parts(upstream_request_head, FakeSend::new(BoxBody::default()));
        *upgrade_request.version_mut() = Version::HTTP_11;

        // @TODO Handle error
        let response = match self.client.request(upgrade_request).upgrade().send().await {
            Ok(res) => res,
            Err(err) => {
                println!("upgrade error: {:?}", err);
                return Err(RouterError::Service(ErrorStatus::from(
                    StatusCode::from_u16(503).unwrap(),
                )));
            }
        };

        let (parts, tunnel) = response.into_parts();
        let (upstream_stream_read, mut upstream_stream_write) = tokio::io::split(PollIoAdapter(tunnel.into_inner()));
        // transform stream read into a stream
        let response_body = BoxBody::new(ResponseBodyStream(upstream_stream_read));
        let response = WebResponse::from_parts(parts, response_body);

        // Future that will copy the data from the downstream to the upstream
        tokio::task::spawn_local(async move {
            let copy_future = copy(&mut downstream_stream_read, &mut upstream_stream_write);

            match copy_future.await {
                Ok(_) => (),
                Err(io) => {
                    if io.kind() != std::io::ErrorKind::UnexpectedEof {
                        println!("Error while copying: {:?}", io);
                    }
                }
            }
        });

        Ok(response)
    }
}

impl<'r, C, B> Service<WebContext<'r, C, B>> for ProxyService
where
    C: Clone + 'static + Unpin,
    B: BodyStream<Chunk = Bytes, Error = BodyError> + Default + 'static + Unpin,
{
    type Response = WebResponse<BoxBody>;
    type Error = RouterError<ErrorStatus>;

    async fn call(&self, mut ctx: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        let (downstream_request_head, body_ext) = ctx.take_request().into_parts();
        let is_upgrade_request = contain_value(&downstream_request_head.headers, header::CONNECTION, "upgrade");

        let downstream_body = if !downstream_request_head.headers.contains_key(header::CONTENT_LENGTH)
            && downstream_request_head.headers.get(header::TRANSFER_ENCODING) != Some(&"chunked".parse().unwrap())
            && downstream_request_head.version < Version::HTTP_2
            && !is_upgrade_request
        {
            FakeSend::new(BoxBody::default())
        } else {
            FakeSend::new(BoxBody::new(body_ext))
        };

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
            upstream_request
                .headers_mut()
                .insert(header::TE, "trailers".parse().unwrap());
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
                    upstream_request.headers_mut().append(
                        header::VIA,
                        format!("HTTP/{} {}", version_str, via.name).parse().unwrap(),
                    );
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

        // @TODO Handle max version
        *upstream_request.version_mut() = Version::HTTP_11;

        let is_upgrade_request = contain_value(&downstream_request_head.headers, header::CONNECTION, "upgrade");

        if is_upgrade_request {
            upstream_request
                .headers_mut()
                .insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));

            for val in downstream_request_head.headers.get_all(header::UPGRADE) {
                upstream_request.headers_mut().append(header::UPGRADE, val.clone());
            }

            return self.upgrade(upstream_request).await;
        }

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
                println!("upstream error: {:?}", err);
                // @TODO handle better error
                return Err(RouterError::Service(ErrorStatus::from(
                    StatusCode::from_u16(503).unwrap(),
                )));
            }
        };

        let (parts, body) = upstream_response.into_parts();

        let mut response = WebResponse::new(BoxBody::new(body));
        *response.status_mut() = parts.status;

        map_headers(
            peer,
            response.headers_mut(),
            parts.version,
            parts.status,
            &parts.headers,
        );

        Ok(response)
    }
}

fn map_headers(
    peer: Rc<HttpPeer>,
    downstream_headers: &mut HeaderMap,
    version: Version,
    status: StatusCode,
    upstream_headers: &HeaderMap,
) {
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
                downstream_headers.append(
                    header::VIA,
                    format!("HTTP/{} {}", via_version_str, via.name).parse().unwrap(),
                );
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

struct FakeSend<B>(xitca_unsafe_collection::fake::FakeSend<B>);

impl<B> FakeSend<B> {
    fn new(body: B) -> Self {
        Self(xitca_unsafe_collection::fake::FakeSend::new(body))
    }
}

impl<B> Stream for FakeSend<B>
where
    B: Stream + Unpin,
{
    type Item = B::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut *self.get_mut().0).poll_next(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl AsyncRead for FakeSend<BoxBody> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match Pin::new(&mut *self.get_mut().0).poll_next(cx) {
            Poll::Ready(Some(chunk)) => match chunk {
                Ok(chunk) => {
                    buf.put_slice(&chunk);

                    Poll::Ready(Ok(()))
                }
                Err(e) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e))),
            },
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub struct ResponseBodyStream(ReadHalf<PollIoAdapter<HttpTunnel>>);

impl Stream for ResponseBodyStream {
    type Item = Result<Bytes, Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = Pin::new(&mut self.get_mut().0);
        // @TODO This is certainly a better way that having to create a buffer and copy data around
        let mut buf = vec![0; 8096];
        let mut read_buf = tokio::io::ReadBuf::new(buf.as_mut());

        match this.poll_read(cx, &mut read_buf) {
            Poll::Ready(Ok(())) => {
                let data = read_buf.filled();
                let bytes = Bytes::copy_from_slice(data);

                if bytes.is_empty() {
                    return Poll::Ready(None);
                }

                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(Error::Io(e)))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}
