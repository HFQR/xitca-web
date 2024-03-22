//! http tunnel handling.

use core::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use futures_sink::Sink;

use super::{
    body::ResponseBody,
    bytes::{Buf, Bytes, BytesMut},
    error::Error,
    http::StatusCode,
    tunnel::{Tunnel, TunnelRequest},
};

pub type HttpTunnelRequest<'a> = TunnelRequest<'a, marker::Connect>;

mod marker {
    pub struct Connect;
}

impl<'a> HttpTunnelRequest<'a> {
    /// Send the request and wait for response asynchronously.
    pub async fn send(self) -> Result<Tunnel<HttpTunnel<'a>>, Error> {
        let res = self.req.send().await?;

        let status = res.status();
        let expect_status = StatusCode::OK;
        if status != expect_status {
            return Err(Error::Std(format!("expecting {expect_status}, got {status}").into()));
        }

        let body = res.res.into_body();
        Ok(Tunnel::new(HttpTunnel {
            buf: BytesMut::new(),
            body,
        }))
    }
}

pub struct HttpTunnel<'b> {
    buf: BytesMut,
    body: ResponseBody<'b>,
}

impl<M> Sink<M> for HttpTunnel<'_>
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

impl Stream for HttpTunnel<'_> {
    type Item = Result<Bytes, Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().body).poll_next(cx).map_err(Into::into)
    }
}
