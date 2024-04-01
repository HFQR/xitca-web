//! http tunnel handling.

use core::{
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
    connection::ConnectionExclusive,
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
            body,
            io: Default::default(),
        }))
    }
}

pub struct HttpTunnel<'b> {
    body: ResponseBody<'b>,
    io: TunnelIo,
}

// io type to bridge AsyncIo trait and h2 body's poll based read/write apis.
#[derive(Default)]
struct TunnelIo {
    write_buf: BytesMut,
    #[cfg(feature = "http2")]
    adaptor: TunnelIoAdaptor,
}

#[cfg(feature = "http2")]
#[derive(Default)]
struct TunnelIoAdaptor {
    write_err: Option<io::Error>,
    write_cap: usize,
    read_buf: Option<Bytes>,
    read_err: Option<io::Error>,
    read_closed: bool,
}

impl Leak for HttpTunnel<'_> {
    type Target = HttpTunnel<'static>;

    fn leak(self) -> Self::Target {
        HttpTunnel {
            body: self.body.into_owned(),
            io: self.io,
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
        if !self.io.write_buf.chunk().is_empty() {
            <Self as Sink<M>>::poll_flush(self, cx)
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(self: Pin<&mut Self>, item: M) -> Result<(), Self::Error> {
        let inner = self.get_mut();
        inner.io.write_buf.extend_from_slice(item.as_ref());
        Ok(())
    }

    #[allow(unreachable_code)]
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        use std::io::Write;

        let inner = self.get_mut();

        let _io: &mut ConnectionExclusive = match inner.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut(),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut(),
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => {
                while !inner.io.write_buf.chunk().is_empty() {
                    ready!(body.poll_send_buf(&mut inner.io.write_buf, _cx))?;
                }
                return Poll::Ready(Ok(()));
            }
            _ => unimplemented!("websocket can only be enabled when http1 or http2 feature is also enabled"),
        };

        let mut io = Pin::new(_io);

        while !inner.io.write_buf.chunk().is_empty() {
            match io.as_mut().get_mut().write(inner.io.write_buf.chunk()) {
                Ok(0) => return Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof).into())),
                Ok(n) => {
                    inner.io.write_buf.advance(n);
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
                xitca_io::io::AsyncIo::poll_shutdown(Pin::new(&mut **body.conn_mut()), cx).map_err(Into::into)
            }
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => {
                xitca_io::io::AsyncIo::poll_shutdown(Pin::new(&mut **body.conn_mut()), cx).map_err(Into::into)
            }
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => {
                body.send_data(xitca_http::bytes::Bytes::new(), true)?;
                Poll::Ready(Ok(()))
            }
            _ => unimplemented!("websocket can only be enabled when http1 or http2 feature is also enabled"),
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

impl AsyncIo for HttpTunnel<'_> {
    async fn ready(&mut self, _interest: Interest) -> io::Result<Ready> {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut().ready(_interest).await,
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut().ready(_interest).await,
            #[cfg(feature = "http2")]
            ResponseBody::H2(_) => core::future::poll_fn(|cx| self.poll_ready(_interest, cx)).await,
            _ => Ok(Ready::ALL),
        }
    }

    fn poll_ready(&mut self, _interest: Interest, _cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut().poll_ready(_interest, _cx),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut().poll_ready(_interest, _cx),
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => self.io.poll_ready(body, _interest, _cx),
            _ => Poll::Ready(Ok(Ready::ALL)),
        }
    }

    fn is_vectored_write(&self) -> bool {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref body) => body.conn().is_vectored_write(),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref body) => body.conn().is_vectored_write(),
            _ => false,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => AsyncIo::poll_shutdown(Pin::new(&mut **body.conn_mut()), _cx),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => AsyncIo::poll_shutdown(Pin::new(&mut **body.conn_mut()), _cx),
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => this.io.poll_shutdown(body, _cx),
            _ => Poll::Ready(Ok(())),
        }
    }
}

impl io::Read for HttpTunnel<'_> {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut().read(_buf),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut().read(_buf),
            #[cfg(feature = "http2")]
            ResponseBody::H2(_) => self.io.read(_buf),
            _ => Ok(0),
        }
    }
}

impl io::Write for HttpTunnel<'_> {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut().write(_buf),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut().write(_buf),
            #[cfg(feature = "http2")]
            ResponseBody::H2(ref mut body) => self.io.write(body, _buf),
            _ => Ok(0),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.body {
            #[cfg(feature = "http1")]
            ResponseBody::H1(ref mut body) => body.conn_mut().flush(),
            #[cfg(feature = "http1")]
            ResponseBody::H1Owned(ref mut body) => body.conn_mut().flush(),
            #[cfg(feature = "http2")]
            ResponseBody::H2(_) => self.io.flush(),
            _ => Ok(()),
        }
    }
}

#[cfg(feature = "http2")]
impl TunnelIo {
    // AsyncIo::poll_ready want this api always return Ok unless the underlying async runtime is gone.
    // all real io error should be stored and emit to user when io::{Read, Write} trait methods are utilized.
    fn poll_ready(
        &mut self,
        body: &mut crate::h2::body::ResponseBody,
        interest: Interest,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<Ready>> {
        let mut ready = Ready::EMPTY;

        if interest.is_readable() {
            if self.adaptor.read_buf.is_some() {
                ready |= Ready::READABLE;
            } else if let Poll::Ready(res) = Pin::new(&mut *body).poll_next(cx) {
                ready |= Ready::READABLE;
                match res {
                    Some(Ok(bytes)) => self.adaptor.read_buf = Some(bytes),
                    Some(Err(e)) => self.adaptor.read_err = Some(io::Error::other(e)),
                    None => self.adaptor.read_closed = true,
                }
            }
        }

        if interest.is_writable() {
            if self.adaptor.write_cap > 0 {
                ready |= Ready::WRITABLE;
            } else {
                body.tx.reserve_capacity(4096);
                if let Poll::Ready(Some(res)) = body.tx.poll_capacity(cx) {
                    ready |= Ready::WRITABLE;
                    match res {
                        Ok(n) => self.adaptor.write_cap += n,
                        Err(e) => self.adaptor.write_err = Some(io::Error::other(e)),
                    }
                }
            }
        }

        if ready.is_empty() {
            Poll::Pending
        } else {
            Poll::Ready(Ok(ready))
        }
    }

    fn poll_shutdown(
        &mut self,
        body: &mut crate::h2::body::ResponseBody,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            body.tx.reserve_capacity(1);
            if let Some(Ok(_)) = ready!(body.tx.poll_capacity(cx)) {
                return Poll::Ready(body.tx.send_data(Bytes::new(), true).map_err(io::Error::other));
            }
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(mut bytes) = self.adaptor.read_buf.take() {
            let len = core::cmp::min(buf.len(), bytes.len());
            buf[..len].copy_from_slice(&bytes[..len]);
            bytes.advance(len);
            if !bytes.is_empty() {
                self.adaptor.read_buf = Some(bytes);
            }
            return Ok(len);
        };

        if let Some(e) = self.adaptor.read_err.take() {
            return Err(e);
        }

        if self.adaptor.read_closed {
            return Ok(0);
        }

        Err(io::ErrorKind::WouldBlock.into())
    }

    fn write(&mut self, body: &mut crate::h2::body::ResponseBody, buf: &[u8]) -> io::Result<usize> {
        if let Some(e) = self.adaptor.write_err.take() {
            return Err(e);
        }

        if self.adaptor.write_cap == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }

        let len = core::cmp::min(buf.len(), self.adaptor.write_cap);

        if len == 0 {
            return Ok(0);
        }

        body.tx
            .send_data(Bytes::copy_from_slice(&buf[..len]), false)
            .map_err(io::Error::other)?;
        self.adaptor.write_cap -= len;

        Ok(len)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.adaptor.write_err.take().map(Err).unwrap_or(Ok(()))
    }
}
