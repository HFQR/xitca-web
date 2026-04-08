use core::{
    fmt, mem,
    pin::Pin,
    task::{Context, Poll, ready},
};

use bytes::{Bytes, BytesMut};
use http::{
    HeaderValue, Request, Response,
    header::{CONTENT_TYPE, HeaderMap, HeaderName},
};
use pin_project_lite::pin_project;
use prost::Message;
use tokio::sync::mpsc::{Receiver, Sender, channel};

use super::{
    codec::Codec,
    error::{GrpcError, ProtocolError},
    status::GrpcStatus,
};

#[cfg(feature = "__body_impl")]
use http_body_alt::{Body, BodyExt, Frame};

#[cfg(feature = "__compress")]
use http_encoding::ContentEncoding;

#[cfg(feature = "__compress")]
const GRPC_ENCODING: HeaderName = HeaderName::from_static("grpc-encoding");

const GRPC_STATUS: HeaderName = HeaderName::from_static("grpc-status");

pin_project! {
    /// Decode a `Body` into a stream of gRPC messages.
    ///
    /// Polls the body for data frames, feeds them into the gRPC codec, and yields
    /// decoded protobuf messages via the [`message`](Self::message) method.
    ///
    /// # Termination
    ///
    /// - `Ok(None)`: The body has ended cleanly with no partial frame remaining.
    /// - `Err(GrpcError)`: A protocol error occurred (incomplete frame, size limit,
    ///   decode error, or body read failure).
    pub struct RequestStream<B> {
        #[pin]
        body: B,
        codec: Codec,
        buf: BytesMut,
    }
}

impl<B> RequestStream<B> {
    pub fn new(headers: &HeaderMap, body: B) -> Self {
        let codec = Codec::from_headers(headers);
        Self {
            body,
            codec,
            buf: BytesMut::new(),
        }
    }

    pub fn with_codec(body: B, codec: Codec) -> Self {
        Self {
            body,
            codec,
            buf: BytesMut::new(),
        }
    }

    #[inline]
    pub fn codec(&self) -> &Codec {
        &self.codec
    }

    #[inline]
    pub fn codec_mut(&mut self) -> &mut Codec {
        &mut self.codec
    }
}

impl<B> RequestStream<B>
where
    B: Body + Unpin,
    B::Data: AsRef<[u8]>,
{
    /// Read the next gRPC message from the body.
    ///
    /// Returns `Ok(None)` when the body has ended cleanly.
    pub async fn message<T: Message + Default>(&mut self) -> Result<Option<T>, GrpcError> {
        loop {
            match self.codec.decode::<T>(&mut self.buf) {
                Ok(Some(msg)) => return Ok(Some(msg)),
                Ok(None) => {}
                Err(e) => return Err(GrpcError::new(GrpcStatus::Internal, e.to_string())),
            }

            match self.body.data().await {
                Some(Ok(data)) => self.buf.extend_from_slice(data.as_ref()),
                Some(Err(_)) => {
                    return Err(GrpcError::new(GrpcStatus::Internal, "body read error"));
                }
                None => {
                    return if self.buf.is_empty() {
                        Ok(None)
                    } else {
                        Err(GrpcError::new(GrpcStatus::Internal, "incomplete grpc frame"))
                    };
                }
            }
        }
    }
}

/// Body type for gRPC responses. Yields encoded gRPC data frames and trailers.
pub struct ResponseBody<T> {
    codec: Codec,
    headers: Option<HeaderMap>,
    trailers: Option<HeaderMap>,
    rx: MessageReceiver<T>,
}

enum MessageReceiver<T> {
    Once(T),
    Repeat(Receiver<Frame<T>>),
    Trailers(Option<HeaderMap>),
    Eof,
}

impl<T> ResponseBody<T> {
    /// Create a response body that yields a single message then trailers.
    pub fn once(msg: T) -> Self {
        Self {
            codec: Codec::new(),
            headers: None,
            trailers: None,
            rx: MessageReceiver::Once(msg),
        }
    }

    /// Create a response body backed by a channel for streaming multiple messages.
    pub fn channel() -> (ResponseSender<T>, Self) {
        let (tx, rx) = channel(8);
        (
            ResponseSender { tx },
            Self {
                codec: Codec::new(),
                headers: None,
                trailers: None,
                rx: MessageReceiver::Repeat(rx),
            },
        )
    }

    /// Set initial metadata (response headers) sent alongside the response.
    pub fn initial_metadata(mut self, headers: HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Set trailing metadata merged into the final trailers frame.
    ///
    /// These are combined with `grpc-status`. Trailers pushed via [`ResponseSender::send_trailers`]
    /// are appended on top.
    pub fn trailing_metadata(mut self, trailers: HeaderMap) -> Self {
        self.trailers = Some(trailers);
        self
    }

    /// Take initial metadata headers, if set.
    pub fn take_initial_metadata(&mut self) -> Option<HeaderMap> {
        self.headers.take()
    }

    /// Convert self into a [`Response`] type where self is as the body.
    /// Take in the request it belongs to for potential compression header preparation.
    pub fn into_response(mut self, req: Request<()>) -> Response<Self> {
        #[cfg(feature = "__compress")]
        let encoding = {
            const GRPC_ACCEPT_ENCODING: HeaderName = HeaderName::from_static("grpc-accept-encoding");
            let encoding = ContentEncoding::from_headers_with(req.headers(), &GRPC_ACCEPT_ENCODING);
            self.codec = self.codec.set_encoding(encoding);
            encoding
        };

        let headers = self.take_initial_metadata();

        let mut parts = req.into_parts().0;
        parts.headers.clear();

        let mut res = Response::new(self);
        *res.headers_mut() = parts.headers;
        *res.extensions_mut() = parts.extensions;

        res.headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/grpc"));

        if let Some(meta) = headers {
            res.headers_mut().extend(meta);
        }

        #[cfg(feature = "__compress")]
        if !matches!(encoding, ContentEncoding::Identity) {
            res.headers_mut().insert(GRPC_ENCODING, encoding.as_header_value());
        }

        res
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Frame<T>>> {
        match &mut self.rx {
            MessageReceiver::Once(_) => {
                let MessageReceiver::Once(msg) = mem::replace(&mut self.rx, MessageReceiver::Trailers(None)) else {
                    unreachable!()
                };
                Poll::Ready(Some(Frame::Data(msg)))
            }
            MessageReceiver::Repeat(rx) => {
                let trailers = match ready!(rx.poll_recv(cx)) {
                    Some(Frame::Trailers(trailers)) => Some(trailers),
                    Some(msg) => return Poll::Ready(Some(msg)),
                    None => None,
                };

                self.rx = MessageReceiver::Trailers(trailers);
                self.poll_recv(cx)
            }
            MessageReceiver::Trailers(..) => {
                let MessageReceiver::Trailers(trailers) = mem::replace(&mut self.rx, MessageReceiver::Eof) else {
                    unreachable!()
                };

                let mut trailers_output = self.trailers.take().unwrap_or_else(|| HeaderMap::with_capacity(1));

                if let Some(trailers) = trailers {
                    trailers_output.extend(trailers);
                }

                trailers_output.insert(GRPC_STATUS, http::header::HeaderValue::from_static("0"));

                Poll::Ready(Some(Frame::Trailers(trailers_output)))
            }
            MessageReceiver::Eof => Poll::Ready(None),
        }
    }

    fn set_end_stream(&mut self) {
        self.rx = MessageReceiver::Eof;
    }
}

impl<T> Body for ResponseBody<T>
where
    T: Message + Unpin,
{
    type Data = Bytes;
    type Error = ProtocolError;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        match this._poll_frame(cx) {
            Poll::Ready(Some(Err(e))) => {
                this.set_end_stream();
                let err = GrpcError::new(GrpcStatus::Internal, e.to_string());
                Poll::Ready(Some(Ok(Frame::Trailers(err.trailers()))))
            }
            poll => poll,
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        matches!(self.rx, MessageReceiver::Eof)
    }
}

impl<T> ResponseBody<T>
where
    T: Message,
{
    fn _poll_frame(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, ProtocolError>>> {
        let opt = match ready!(self.poll_recv(cx)) {
            Some(Frame::Data(msg)) => {
                let mut buf = BytesMut::new();
                self.codec.encode(&msg, &mut buf)?;
                Some(Ok(Frame::Data(buf.freeze())))
            }
            Some(Frame::Trailers(trailers)) => Some(Ok(Frame::Trailers(trailers))),
            None => None,
        };

        Poll::Ready(opt)
    }
}

/// Send gRPC messages to a [`ResponseBody`].
pub struct ResponseSender<T> {
    tx: Sender<Frame<T>>,
}

impl<T> ResponseSender<T> {
    /// Send a message to the response body.
    pub async fn send_message(&mut self, msg: T) -> Result<(), T> {
        self.tx.send(Frame::Data(msg)).await.map_err(|e| {
            let Frame::Data(msg) = e.0 else { unreachable!() };
            msg
        })
    }

    /// Send trailing metadata. Consumes the sender as no more messages can follow.
    pub async fn send_trailers(self, trailers: HeaderMap) -> Result<(), HeaderMap> {
        self.tx.send(Frame::Trailers(trailers)).await.map_err(|e| {
            let Frame::Trailers(trailers) = e.0 else { unreachable!() };
            trailers
        })
    }
}

impl<T> fmt::Debug for ResponseSender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseSender").finish()
    }
}
