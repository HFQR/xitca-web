//! WebSocket protocol using high level API that operate over `futures::Stream` trait.
//!
//! `http` crate is used as both Http request input and response output.
//!
//! To setup a WebSocket, first perform the WebSocket handshake then on success convert request's
//! body into a `DecodeStream` stream and then use `EncodeStream` to communicate with the peer.
//!
//! # Examples:
//! ```rust
//! # use std::{pin::Pin, task::{Context, Poll}};
//! # use http::{Request, Response, header, Method};
//! # use http_ws::{handshake, DecodeStream, Message};
//! # use futures_util::stream::{Stream, StreamExt};
//! #
//! # fn ws() -> Response<http_ws::EncodeStream> {
//! #
//! # struct DummyRequestBody;
//! #
//! # impl Stream for DummyRequestBody {
//! #   type Item = Result<Vec<u8>, ()>;
//! #   fn poll_next(self:Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
//! #        Poll::Ready(Some(Ok(vec![1, 2, 3])))
//! #    }
//! # }
//!
//! // an incoming http request.
//! let request = http::Request::get("/")
//!     .header(header::UPGRADE, header::HeaderValue::from_static("websocket"))
//!     .header(header::CONNECTION, header::HeaderValue::from_static("upgrade"))
//!     .header(header::SEC_WEBSOCKET_VERSION, header::HeaderValue::from_static("13"))
//!     .header(header::SEC_WEBSOCKET_KEY, header::HeaderValue::from_static("some_key"))
//!     .body(DummyRequestBody)
//!     .unwrap();
//!
//! let method = request.method();
//! let headers = request.headers();
//!
//! // handshake with request and return a response builder on success.
//! let response_builder = handshake(method, headers).unwrap();
//!
//! // extract request body and construct decode stream.
//! let body = request.into_body();
//! let mut decode = DecodeStream::new(body);
//!
//! // generate an encode stream and a sender that for adding message to it.
//! let (encode_tx, encode) = decode.encode_stream();
//!
//! // attach encode stream to builder to construct a full response.
//! let response = response_builder.body(encode).unwrap();
//!
//! // spawn an async task that decode request streaming body and send message to encode stream
//! // that would encode and sending response.
//! tokio::spawn(async move {
//!     while let Some(Ok(msg)) = decode.next().await {
//!         match msg {
//!             // echo back text and ping messages and ignore others.
//!             Message::Text(txt) => encode_tx
//!                 .send(Message::Text(txt))
//!                 .await
//!                 .unwrap(),
//!             Message::Ping(ping) => encode_tx
//!                 .send(Message::Pong(ping))
//!                 .await
//!                 .unwrap(),
//!             _ => {}
//!         }   
//!     }
//! });
//!
//! response
//! # }
//! ```

use http::{
    header::{
        HeaderMap, HeaderName, HeaderValue, ALLOW, CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY,
        SEC_WEBSOCKET_VERSION, UPGRADE,
    },
    request::Request,
    response::{Builder, Response},
    uri::Uri,
    Method, StatusCode, Version,
};

mod codec;
mod error;
mod frame;
mod mask;
mod proto;

pub use self::{
    codec::{Codec, Item, Message},
    error::{HandshakeError, ProtocolError},
    proto::{hash_key, CloseCode, CloseReason, OpCode},
};

#[allow(clippy::declare_interior_mutable_const)]
mod const_header {
    use super::{HeaderName, HeaderValue};

    pub(super) const PROTOCOL: HeaderName = HeaderName::from_static("protocol");

    pub(super) const WEBSOCKET: HeaderValue = HeaderValue::from_static("websocket");
    pub(super) const UPGRADE_VALUE: HeaderValue = HeaderValue::from_static("upgrade");
    pub(super) const SEC_WEBSOCKET_VERSION_VALUE: HeaderValue = HeaderValue::from_static("13");
}

use const_header::*;

impl From<HandshakeError> for Builder {
    fn from(e: HandshakeError) -> Self {
        match e {
            HandshakeError::GetMethodRequired => Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .header(ALLOW, "GET"),

            _ => Response::builder().status(StatusCode::BAD_REQUEST),
        }
    }
}

/// Prepare a [Request] with given [Uri] and [Version]  for websocket connection.
/// Only [Version::HTTP_11] and [Version::HTTP_2] are supported.
/// After process the request would be ready to be sent to server.
pub fn client_request_from_uri<U, E>(uri: U, version: Version) -> Result<Request<()>, E>
where
    Uri: TryFrom<U, Error = E>,
{
    let uri = uri.try_into()?;

    let mut req = Request::new(());
    *req.uri_mut() = uri;
    *req.version_mut() = version;

    match version {
        Version::HTTP_11 => {
            req.headers_mut().insert(UPGRADE, WEBSOCKET);
            req.headers_mut().insert(CONNECTION, UPGRADE_VALUE);

            // generate 24 bytes base64 encoded random key.
            let input = rand::random::<[u8; 16]>();
            let mut output = [0u8; 24];

            #[allow(clippy::needless_borrow)] // clippy dumb.
            let n = base64::encode_engine_slice(&input, &mut output, &base64::engine::DEFAULT_ENGINE);
            assert_eq!(n, output.len());

            req.headers_mut()
                .insert(SEC_WEBSOCKET_KEY, HeaderValue::from_bytes(&output).unwrap());
        }
        Version::HTTP_2 => {
            *req.method_mut() = Method::CONNECT;
            req.headers_mut().insert(PROTOCOL, WEBSOCKET);
        }
        _ => {}
    }

    req.headers_mut()
        .insert(SEC_WEBSOCKET_VERSION, SEC_WEBSOCKET_VERSION_VALUE);

    Ok(req)
}

/// Verify HTTP/1.1 WebSocket handshake request and create handshake response.
pub fn handshake(method: &Method, headers: &HeaderMap) -> Result<Builder, HandshakeError> {
    let key = verify_handshake(method, headers)?;
    let builder = handshake_response(key);
    Ok(builder)
}

/// Verify HTTP/2 WebSocket handshake request and create handshake response.
pub fn handshake_h2(method: &Method, headers: &HeaderMap) -> Result<Builder, HandshakeError> {
    // Check for method
    if method != Method::CONNECT {
        return Err(HandshakeError::ConnectMethodRequired);
    }

    ws_version_check(headers)?;

    Ok(Response::builder().status(StatusCode::OK))
}

/// Verify WebSocket handshake request and return `SEC_WEBSOCKET_KEY` header value as `&[u8]`
fn verify_handshake<'a>(method: &'a Method, headers: &'a HeaderMap) -> Result<&'a [u8], HandshakeError> {
    // Check for method
    if method != Method::GET {
        return Err(HandshakeError::GetMethodRequired);
    }

    // Check for "Upgrade" header
    let has_upgrade_hd = headers
        .get(UPGRADE)
        .and_then(|hdr| hdr.to_str().ok())
        .filter(|s| s.to_ascii_lowercase().contains("websocket"))
        .is_some();

    if !has_upgrade_hd {
        return Err(HandshakeError::NoWebsocketUpgrade);
    }

    // Check for "Connection" header
    let has_connection_hd = headers
        .get(CONNECTION)
        .and_then(|hdr| hdr.to_str().ok())
        .filter(|s| s.to_ascii_lowercase().contains("upgrade"))
        .is_some();

    if !has_connection_hd {
        return Err(HandshakeError::NoConnectionUpgrade);
    }

    ws_version_check(headers)?;

    // check client handshake for validity
    let value = headers.get(SEC_WEBSOCKET_KEY).ok_or(HandshakeError::BadWebsocketKey)?;

    Ok(value.as_bytes())
}

/// Create WebSocket handshake response.
///
/// This function returns handshake `http::response::Builder`, ready to send to peer.
fn handshake_response(key: &[u8]) -> Builder {
    let key = hash_key(key);

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, WEBSOCKET)
        .header(CONNECTION, UPGRADE_VALUE)
        .header(
            SEC_WEBSOCKET_ACCEPT,
            // key is known to be header value safe ascii
            HeaderValue::from_bytes(&key).unwrap(),
        )
}

// check supported version
fn ws_version_check(headers: &HeaderMap) -> Result<(), HandshakeError> {
    let value = headers
        .get(SEC_WEBSOCKET_VERSION)
        .ok_or(HandshakeError::NoVersionHeader)?;

    if value != "13" && value != "8" && value != "7" {
        Err(HandshakeError::UnsupportedVersion)
    } else {
        Ok(())
    }
}

#[cfg(feature = "stream")]
pub mod stream;

#[cfg(feature = "stream")]
pub use self::stream::{DecodeError, DecodeStream, EncodeStream};

#[cfg(feature = "stream")]
pub type WsOutput<B, E> = (
    DecodeStream<B, E>,
    Response<EncodeStream>,
    tokio::sync::mpsc::Sender<Message>,
);

#[cfg(feature = "stream")]
/// A shortcut for generating a set of response types with given [Request] and `<Body>` type.
///
/// `<Body>` must be a type impl [futures_core::Stream] trait with `Result<T: AsRef<[u8]>, E>`
/// as `Stream::Item` associated type.
///
/// # Examples:
/// ```rust
/// # use std::pin::Pin;
/// # use std::task::{Context, Poll};
/// # use http::{header, Request};
/// # use futures_core::Stream;
/// # #[derive(Default)]
/// # struct DummyRequestBody;
/// #
/// # impl Stream for DummyRequestBody {
/// #   type Item = Result<Vec<u8>, ()>;
/// #   fn poll_next(self:Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
/// #        Poll::Ready(Some(Ok(vec![1, 2, 3])))
/// #    }
/// # }
/// // an incoming http request.
/// let mut req = Request::get("/")
///     .header(header::UPGRADE, header::HeaderValue::from_static("websocket"))
///     .header(header::CONNECTION, header::HeaderValue::from_static("upgrade"))
///     .header(header::SEC_WEBSOCKET_VERSION, header::HeaderValue::from_static("13"))
///     .header(header::SEC_WEBSOCKET_KEY, header::HeaderValue::from_static("some_key"))
///     .body(())
///     .unwrap();
///
/// let (decoded_stream, http_response, encode_stream_sink) = http_ws::ws(&mut req, DummyRequestBody).unwrap();
/// ```
pub fn ws<ReqB, B, T, E>(req: &Request<ReqB>, body: B) -> Result<WsOutput<B, E>, HandshakeError>
where
    B: futures_core::Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    let builder = match req.version() {
        Version::HTTP_2 => handshake_h2(req.method(), req.headers())?,
        _ => handshake(req.method(), req.headers())?,
    };

    let decode = DecodeStream::new(body);
    let (tx, encode) = decode.encode_stream();

    let res = builder
        .body(encode)
        .expect("handshake function failed to generate correct Response Builder");

    Ok((decode, res, tx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handshake() {
        let req = Request::builder().method(Method::POST).body(()).unwrap();
        assert_eq!(
            HandshakeError::GetMethodRequired,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = Request::builder().body(()).unwrap();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = Request::builder()
            .header(UPGRADE, HeaderValue::from_static("test"))
            .body(())
            .unwrap();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = Request::builder().header(UPGRADE, WEBSOCKET).body(()).unwrap();
        assert_eq!(
            HandshakeError::NoConnectionUpgrade,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = Request::builder()
            .header(UPGRADE, WEBSOCKET)
            .header(CONNECTION, UPGRADE_VALUE)
            .body(())
            .unwrap();
        assert_eq!(
            HandshakeError::NoVersionHeader,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = Request::builder()
            .header(UPGRADE, WEBSOCKET)
            .header(CONNECTION, UPGRADE_VALUE)
            .header(SEC_WEBSOCKET_VERSION, HeaderValue::from_static("5"))
            .body(())
            .unwrap();
        assert_eq!(
            HandshakeError::UnsupportedVersion,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let builder = || {
            Request::builder()
                .header(UPGRADE, WEBSOCKET)
                .header(CONNECTION, UPGRADE_VALUE)
                .header(SEC_WEBSOCKET_VERSION, SEC_WEBSOCKET_VERSION_VALUE)
        };

        let req = builder().body(()).unwrap();
        assert_eq!(
            HandshakeError::BadWebsocketKey,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = builder()
            .header(SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION_VALUE)
            .body(())
            .unwrap();
        let key = verify_handshake(req.method(), req.headers()).unwrap();
        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake_response(key).body(()).unwrap().status()
        );
    }

    #[test]
    fn test_ws_error_http_response() {
        let res = Builder::from(HandshakeError::GetMethodRequired).body(()).unwrap();
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
        let res = Builder::from(HandshakeError::NoWebsocketUpgrade).body(()).unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let res = Builder::from(HandshakeError::NoConnectionUpgrade).body(()).unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let res = Builder::from(HandshakeError::NoVersionHeader).body(()).unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let res = Builder::from(HandshakeError::UnsupportedVersion).body(()).unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let res = Builder::from(HandshakeError::BadWebsocketKey).body(()).unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
}
