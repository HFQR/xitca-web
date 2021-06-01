//! WebSocket protocol using high level API that operate over `futures::Stream` trait.
//!
//! `http` crate is used as both Http request input and response output.
//!
//! To setup a WebSocket, first perform the WebSocket handshake then on success convert request's
//! body into a `DecodeStream` stream and then use `EncodeStream` to communicate with the peer.
//!
//! # Examples:
//! ```rust
//! # use std::pin::Pin;
//! # use std::task::{Context, Poll};
//! use http::{Request, Response, header, Method};
//! use http_ws::{handshake, DecodeStream, Message};
//! # use futures_core::Stream;
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
//! tokio::task::spawn_local(async move {
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
    header::{self, HeaderValue},
    response::{Builder, Response},
    HeaderMap, Method, StatusCode,
};

mod codec;
mod error;
mod frame;
mod mask;
mod proto;

pub use self::codec::{Codec, Item, Message};
pub use self::error::{HandshakeError, ProtocolError};
pub use self::proto::{hash_key, CloseCode, CloseReason, OpCode};

impl From<HandshakeError> for Builder {
    fn from(e: HandshakeError) -> Self {
        match e {
            HandshakeError::GetMethodRequired => Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .header(header::ALLOW, "GET"),

            _ => Response::builder().status(StatusCode::BAD_REQUEST),
        }
    }
}

/// Verify WebSocket handshake request and create handshake response.
pub fn handshake(method: &Method, headers: &HeaderMap) -> Result<Builder, HandshakeError> {
    let key = verify_handshake(method, headers)?;
    let builder = handshake_response(key);
    Ok(builder)
}

/// Verify WebSocket handshake request and return `SEC_WEBSOCKET_KEY` header value as `&[u8]`
fn verify_handshake<'a>(method: &'a Method, headers: &'a HeaderMap) -> Result<&'a [u8], HandshakeError> {
    // WebSocket accepts only GET
    if method != Method::GET {
        return Err(HandshakeError::GetMethodRequired);
    }

    // Check for "Upgrade" header
    let has_upgrade_hd = headers
        .get(header::UPGRADE)
        .and_then(|hdr| hdr.to_str().ok())
        .filter(|s| s.to_ascii_lowercase().contains("websocket"))
        .is_some();

    if !has_upgrade_hd {
        return Err(HandshakeError::NoWebsocketUpgrade);
    }

    // Check for "Connection" header
    let has_connection_hd = headers
        .get(header::CONNECTION)
        .and_then(|hdr| hdr.to_str().ok())
        .filter(|s| s.to_ascii_lowercase().contains("upgrade"))
        .is_some();

    if !has_connection_hd {
        return Err(HandshakeError::NoConnectionUpgrade);
    }

    // check supported version
    let value = headers
        .get(header::SEC_WEBSOCKET_VERSION)
        .ok_or(HandshakeError::NoVersionHeader)?;

    if value != "13" && value != "8" && value != "7" {
        return Err(HandshakeError::UnsupportedVersion);
    }

    // check client handshake for validity
    let value = headers
        .get(header::SEC_WEBSOCKET_KEY)
        .ok_or(HandshakeError::BadWebsocketKey)?;

    Ok(value.as_bytes())
}

/// Create WebSocket handshake response.
///
/// This function returns handshake `http::response::Builder`, ready to send to peer.
fn handshake_response(key: &[u8]) -> Builder {
    let key = proto::hash_key(key);

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::UPGRADE, "websocket")
        .header(header::TRANSFER_ENCODING, "chunked")
        .header(
            header::SEC_WEBSOCKET_ACCEPT,
            // key is known to be header value safe ascii
            HeaderValue::from_bytes(&key).unwrap(),
        )
}

#[cfg(feature = "stream")]
mod stream;

#[cfg(feature = "stream")]
pub use self::stream::{DecodeError, DecodeStream, EncodeStream};

#[cfg(feature = "stream")]
pub type WsOutput<B> = (
    DecodeStream<B>,
    Response<EncodeStream>,
    tokio::sync::mpsc::Sender<Message>,
);

#[cfg(feature = "stream")]
/// A shortcut for generating a set of response types with given [Request](http::Request).
pub fn ws<B, T, E>(req: http::Request<B>) -> Result<WsOutput<B>, HandshakeError>
where
    B: futures_core::Stream<Item = Result<T, E>>,
    T: AsRef<[u8]>,
{
    let builder = handshake(req.method(), req.headers())?;

    let body = req.into_body();

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

    use http::Request;

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
            .header(header::UPGRADE, header::HeaderValue::from_static("test"))
            .body(())
            .unwrap();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = Request::builder()
            .header(header::UPGRADE, header::HeaderValue::from_static("websocket"))
            .body(())
            .unwrap();
        assert_eq!(
            HandshakeError::NoConnectionUpgrade,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = Request::builder()
            .header(header::UPGRADE, header::HeaderValue::from_static("websocket"))
            .header(header::CONNECTION, header::HeaderValue::from_static("upgrade"))
            .body(())
            .unwrap();
        assert_eq!(
            HandshakeError::NoVersionHeader,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = Request::builder()
            .header(header::UPGRADE, header::HeaderValue::from_static("websocket"))
            .header(header::CONNECTION, header::HeaderValue::from_static("upgrade"))
            .header(header::SEC_WEBSOCKET_VERSION, header::HeaderValue::from_static("5"))
            .body(())
            .unwrap();
        assert_eq!(
            HandshakeError::UnsupportedVersion,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let builder = || {
            Request::builder()
                .header(header::UPGRADE, header::HeaderValue::from_static("websocket"))
                .header(header::CONNECTION, header::HeaderValue::from_static("upgrade"))
                .header(header::SEC_WEBSOCKET_VERSION, header::HeaderValue::from_static("13"))
        };

        let req = builder().body(()).unwrap();
        assert_eq!(
            HandshakeError::BadWebsocketKey,
            verify_handshake(req.method(), req.headers()).unwrap_err(),
        );

        let req = builder()
            .header(header::SEC_WEBSOCKET_KEY, header::HeaderValue::from_static("13"))
            .body(())
            .unwrap();
        let key = verify_handshake(req.method(), req.headers()).unwrap();
        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake_response(key).body(()).unwrap().status()
        );
    }

    #[test]
    fn test_wserror_http_response() {
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
