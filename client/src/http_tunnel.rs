//! http tunnel handling.

use core::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::io;

use futures_core::stream::Stream;
use futures_sink::Sink;
use xitca_io::io::{AsyncIo, Interest, Ready};

use super::{
    body::ResponseBody,
    bytes::{Buf, Bytes, BytesMut},
    connection::Connection,
    error::{Error, ErrorResponse},
    http::StatusCode,
    tunnel::{Leak, Tunnel, TunnelRequest},
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
            return Err(Error::from(ErrorResponse {
                expect_status,
                status,
                description: "connect tunnel can't be established",
            }));
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

impl Leak for HttpTunnel<'_> {
    type Target = HttpTunnel<'static>;

    fn leak(self) -> Self::Target {
        HttpTunnel {
            buf: self.buf,
            body: self.body.into_owned(),
        }
    }
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

    #[allow(unreachable_code)]
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        use std::io::Write;

        let inner = self.get_mut();

        let _io: &mut Connection = match inner.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut(),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut(),
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => {
                while !inner.buf.chunk().is_empty() {
                    ready!(body.poll_send_buf(&mut inner.buf, _cx))?;
                }
                return Poll::Ready(Ok(()));
            }
            _ => unimplemented!("websocket can only be enabled when http1 or http2 feature is also enabled"),
        };

        let mut io = Pin::new(_io);

        while !inner.buf.chunk().is_empty() {
            match io.as_mut().get_mut().write(inner.buf.chunk()) {
                Ok(0) => return Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())),
                Ok(n) => {
                    inner.buf.advance(n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ready!(io.as_mut().poll_ready(Interest::WRITABLE, _cx))?;
                }
                Err(e) => return Poll::Ready(Err(e.into())),
            }
        }

        while let Err(e) = io.as_mut().get_mut().flush() {
            if e.kind() != io::ErrorKind::WouldBlock {
                return Poll::Ready(Err(e.into()));
            }
            ready!(io.as_mut().poll_ready(Interest::WRITABLE, _cx))?;
        }

        Poll::Ready(Ok(()))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(<Self as Sink<M>>::poll_flush(self.as_mut(), cx))?;
        match self.get_mut().body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => {
                ready!(xitca_io::io::AsyncIo::poll_shutdown(
                    Pin::new(&mut **body.conn_mut()),
                    cx
                ))?;
            }
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => {
                ready!(xitca_io::io::AsyncIo::poll_shutdown(
                    Pin::new(&mut **body.conn_mut()),
                    cx
                ))?;
            }
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => {
                body.send_data(xitca_http::bytes::Bytes::new(), true)?;
            }
            _ => unimplemented!("websocket can only be enabled when http1 or http2 feature is also enabled"),
        };

        Poll::Ready(Ok(()))
    }
}

impl Stream for HttpTunnel<'_> {
    type Item = Result<Bytes, Error>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().body).poll_next(cx).map_err(Into::into)
    }
}

impl AsyncIo for HttpTunnel<'_> {
    fn ready(&self, interest: Interest) -> impl Future<Output = io::Result<Ready>> + Send {
        let conn: &Connection = match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref body) => body.conn(),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref body) => body.conn(),
            _ => unimplemented!("AsyncIo through HttpTunnel only supports http/1"),
        };

        conn.ready(interest)
    }

    fn poll_ready(&self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<tokio::io::Ready>> {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref body) => body.conn().poll_ready(interest, cx),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref body) => body.conn().poll_ready(interest, cx),
            _ => unimplemented!("AsyncIo through HttpTunnel only supports http/1"),
        }
    }

    fn is_vectored_write(&self) -> bool {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref body) => body.conn().is_vectored_write(),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref body) => body.conn().is_vectored_write(),
            _ => unimplemented!("AsyncIo through HttpTunnel only supports http/1"),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut().body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => AsyncIo::poll_shutdown(Pin::new(&mut **body.conn_mut()), cx),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => AsyncIo::poll_shutdown(Pin::new(&mut **body.conn_mut()), cx),
            _ => unimplemented!("AsyncIo through HttpTunnel only supports http/1"),
        }
    }
}

impl io::Read for HttpTunnel<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut().read(buf),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut().read(buf),
            _ => unimplemented!("Read through HttpTunnel only supports http/1"),
        }
    }
}

impl io::Write for HttpTunnel<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut().write(buf),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut().write(buf),
            _ => unimplemented!("Write through HttpTunnel only supports http/1"),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut().flush(),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut().flush(),
            _ => unimplemented!("Write through HttpTunnel only supports http/1"),
        }
    }
}
