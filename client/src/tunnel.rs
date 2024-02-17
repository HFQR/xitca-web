use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{ready, Context, Poll},
    time::Duration,
};

use std::sync::Mutex;

use futures_core::stream::Stream;
use futures_sink::Sink;

use crate::{
    body::ResponseBody,
    bytes::{Buf, Bytes, BytesMut},
    error::Error,
    http::{Method, StatusCode, Version},
    request::RequestBuilder,
};

/// new type of [RequestBuilder] with extended functionality for tunnel handling.
pub struct TunnelRequest<'a, M = marker::Connect> {
    pub(crate) req: RequestBuilder<'a>,
    _marker: PhantomData<M>,
}

mod marker {
    pub struct Connect;
}

/// new type of [RequestBuilder] with extended functionality for tunnel handling.

impl<'a, M> Deref for TunnelRequest<'a, M> {
    type Target = RequestBuilder<'a>;

    fn deref(&self) -> &Self::Target {
        &self.req
    }
}

impl<'a, M> DerefMut for TunnelRequest<'a, M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.req
    }
}

impl<'a, M> TunnelRequest<'a, M> {
    pub(super) fn new(req: RequestBuilder<'a>) -> Self {
        Self {
            req,
            _marker: PhantomData,
        }
    }

    /// Set HTTP method of this request.
    pub fn method(mut self, method: Method) -> Self {
        self.req = self.req.method(method);
        self
    }

    #[doc(hidden)]
    /// Set HTTP version of this request.
    ///
    /// By default request's HTTP version depends on network stream
    pub fn version(mut self, version: Version) -> Self {
        self.req = self.req.version(version);
        self
    }

    /// Set timeout of this request.
    ///
    /// The value passed would override global [ClientBuilder::set_request_timeout].
    ///
    /// [ClientBuilder::set_request_timeout]: crate::ClientBuilder::set_request_timeout
    pub fn timeout(mut self, dur: Duration) -> Self {
        self.req = self.req.timeout(dur);
        self
    }
}

impl<'a> TunnelRequest<'a> {
    /// Send the request and wait for response asynchronously.
    pub async fn send(self) -> Result<Tunnel<'a>, Error> {
        let res = self.req.send().await?;

        let status = res.status();
        let expect_status = StatusCode::OK;
        if status != expect_status {
            return Err(Error::Std(format!("expecting {expect_status}, got {status}").into()));
        }

        let body = res.res.into_body();
        Tunnel::try_from_body(body)
    }
}

/// sender part of tunneled connection.
/// [Sink] trait is used to asynchronously send message.
pub struct TunnelSink<'a, 'b>(&'a Tunnel<'b>);

impl<M> Sink<M> for TunnelSink<'_, '_>
where
    M: AsRef<[u8]>,
{
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        <TunnelInner<'_> as Sink<M>>::poll_ready(Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()), cx)
    }

    fn start_send(self: Pin<&mut Self>, item: M) -> Result<(), Self::Error> {
        Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()).start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        <TunnelInner<'_> as Sink<M>>::poll_flush(Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()), cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        <TunnelInner<'_> as Sink<M>>::poll_close(Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()), cx)
    }
}

/// sender part of tunnel connection.
/// [Stream] trait is used to asynchronously receive message.
pub struct TunnelStream<'a, 'b>(&'a Tunnel<'b>);

impl Stream for TunnelStream<'_, '_> {
    type Item = Result<Bytes, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut *self.get_mut().0.inner.lock().unwrap()).poll_next(cx)
    }
}

/// A unified tunnel that can be used as both sender/receiver.
///
/// * This type can not do concurrent message handling which means send always block receive
/// and vice versa.
pub struct Tunnel<'c> {
    inner: Mutex<TunnelInner<'c>>,
}

impl<'a> Tunnel<'a> {
    pub(crate) fn try_from_body(body: ResponseBody<'a>) -> Result<Self, Error> {
        Ok(Self {
            inner: Mutex::new(TunnelInner {
                buf: BytesMut::new(),
                body,
            }),
        })
    }

    /// Split into a sink and reader pair that can be used for concurrent read/write
    /// message to tunnel connection.
    #[inline]
    pub fn split(&self) -> (TunnelSink<'_, 'a>, TunnelStream<'_, 'a>) {
        (TunnelSink(self), TunnelStream(self))
    }

    fn get_mut_pinned_inner(self: Pin<&mut Self>) -> Pin<&mut TunnelInner<'a>> {
        Pin::new(self.get_mut().inner.get_mut().unwrap())
    }
}

impl<M> Sink<M> for Tunnel<'_>
where
    M: AsRef<[u8]>,
{
    type Error = Error;

    #[inline]
    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        <TunnelInner<'_> as Sink<M>>::poll_ready(self.get_mut_pinned_inner(), cx)
    }

    #[inline]
    fn start_send(self: Pin<&mut Self>, item: M) -> Result<(), Self::Error> {
        self.get_mut_pinned_inner().start_send(item)
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        <TunnelInner<'_> as Sink<M>>::poll_flush(self.get_mut_pinned_inner(), cx)
    }

    #[inline]
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        <TunnelInner<'_> as Sink<M>>::poll_close(self.get_mut_pinned_inner(), cx)
    }
}

impl Stream for Tunnel<'_> {
    type Item = Result<Bytes, Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut_pinned_inner().poll_next(cx)
    }
}

struct TunnelInner<'b> {
    buf: BytesMut,
    body: ResponseBody<'b>,
}

impl<M> Sink<M> for TunnelInner<'_>
where
    M: AsRef<[u8]>,
{
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // TODO: set up a meaningful backpressure limit for send buf.
        if !self.buf.chunk().is_empty() {
            <Self as Sink<M>>::poll_flush(self, cx)
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(self: Pin<&mut Self>, item: M) -> Result<(), Self::Error> {
        let inner = self.get_mut();
        inner.buf.extend_from_slice(item.as_ref());
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let inner = self.get_mut();

        match inner.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => {
                use std::io;
                use tokio::io::AsyncWrite;
                while !inner.buf.chunk().is_empty() {
                    match ready!(Pin::new(&mut **body.conn()).poll_write(_cx, inner.buf.chunk()))? {
                        0 => return Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())),
                        n => inner.buf.advance(n),
                    }
                }

                Pin::new(&mut **body.conn()).poll_flush(_cx).map_err(Into::into)
            }
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => {
                while !inner.buf.chunk().is_empty() {
                    ready!(body.poll_send_buf(&mut inner.buf, _cx))?;
                }

                Poll::Ready(Ok(()))
            }
            _ => panic!("tunnel can only be enabled when http1 or http2 feature is also enabled"),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(<Self as Sink<M>>::poll_flush(self.as_mut(), cx))?;
        match self.get_mut().body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => {
                tokio::io::AsyncWrite::poll_shutdown(Pin::new(&mut **body.conn()), cx).map_err(Into::into)
            }
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => {
                body.send_data(Bytes::new(), true)?;
                Poll::Ready(Ok(()))
            }
            _ => panic!("tunnel can only be enabled when http1 or http2 feature is also enabled"),
        }
    }
}

impl Stream for TunnelInner<'_> {
    type Item = Result<Bytes, Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().body).poll_next(cx).map_err(Into::into)
    }
}
