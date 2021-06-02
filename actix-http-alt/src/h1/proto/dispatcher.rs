use std::{
    future::Future,
    io,
    marker::PhantomData,
    pin::Pin,
    task::{self, Poll},
};

use actix_server_alt::net::TcpStream;
use actix_service_alt::Service;
use bytes::{Buf, Bytes, BytesMut};
use futures_core::{ready, stream::Stream};
use http::{response::Parts, Request, Response};
use pin_project_lite::pin_project;
use tokio::io::AsyncWrite;

use crate::body::ResponseBody;
use crate::error::BodyError;
use crate::flow::HttpFlow;
use crate::h1::{
    body::{RequestBody, RequestBodySender},
    error::Error,
};
use crate::response::ResponseError;
use crate::util::{date::DateTask, poll_fn::poll_fn};

use super::context::Context;
use super::decode::{RequestBodyDecoder, RequestBodyItem};
use super::encode::TransferEncoding;
use super::state::State;

pub(crate) struct Dispatcher<'a, S, B, X, U> {
    io: Io<'a>,
    ctx: Context<'a>,
    error: Option<Error>,
    flow: &'a HttpFlow<S, X, U>,
    _phantom: PhantomData<B>,
}

struct Io<'a> {
    io: &'a mut TcpStream,
    state: State,
    read_buf: ReadBuffer,
    write_buf: BytesMut,
}

impl Io<'_> {
    fn try_read(&mut self) -> Result<(), Error> {
        let read_buf = &mut self.read_buf;
        read_buf.advance(false);

        loop {
            match self.io.try_read_buf(read_buf.buf()) {
                Ok(0) => return Err(Error::Closed),
                Ok(_) => read_buf.advance(true),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(ref e) if e.kind() == io::ErrorKind::ConnectionReset => {
                    return Err(Error::Closed);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Return true when write is blocked and need wait.
    /// Return false when write is finished.(Did not blocked)
    fn try_write(&mut self) -> Result<bool, Error> {
        let mut written = 0;
        let len = self.write_buf.len();

        while written < len {
            match self.io.try_write(&self.write_buf[written..]) {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => written += n,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.write_buf.advance(written);
                    return Ok(true);
                }
                Err(e) => return Err(e.into()),
            }
        }

        self.write_buf.clear();

        Ok(false)
    }

    /// Read once and fill read buffer.
    #[inline(always)]
    async fn read(&mut self) -> Result<(), Error> {
        self.io.readable().await?;
        self.try_read()
    }

    /// drain write buffer and flush the io.
    #[inline(always)]
    async fn drain_write(&mut self) -> Result<(), Error> {
        while self.try_write()? {
            self.io.writable().await?;
        }
        poll_fn(|cx| Pin::new(&mut *self.io).poll_flush(cx)).await?;
        Ok(())
    }
}

impl<'a, S, B, E, X, U> Dispatcher<'a, S, B, X, U>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody<B>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: Service<Request<RequestBody>, Response = Request<RequestBody>> + 'static,
    X::Error: ResponseError<S::Response>,

    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    pub(crate) fn new(io: &'a mut TcpStream, flow: &'a HttpFlow<S, X, U>, date: &'a DateTask) -> Self {
        let io = Io {
            io,
            state: State::new(),
            read_buf: ReadBuffer::new(),
            write_buf: BytesMut::new(),
        };

        Self {
            io,
            ctx: Context::new(date.get()),
            error: None,
            flow,
            _phantom: PhantomData,
        }
    }

    pub(crate) async fn run(&mut self) -> Result<(), Error> {
        loop {
            while let Some((req, mut body_handle)) = self.decode_head()? {
                let res = if self.ctx.is_expect() {
                    match self.flow.expect.call(req).await {
                        Ok(req) => {
                            // encode continue
                            self.ctx.encode_continue(&mut self.io.write_buf);

                            // use drain write to make sure continue is sent to client.
                            // the world can wait until it happens.
                            self.io.drain_write().await?;

                            let fut = self.flow.service.call(req);

                            RequestHandler {
                                fut,
                                body_handle: body_handle.as_mut(),
                                io: &mut self.io,
                                _body: PhantomData,
                            }
                            .await?
                        }
                        Err(e) => ResponseError::response_error(e),
                    }
                } else {
                    let fut = self.flow.service.call(req);

                    RequestHandler {
                        fut,
                        body_handle: body_handle.as_mut(),
                        io: &mut self.io,
                        _body: PhantomData,
                    }
                    .await?
                };

                let (parts, res_body) = res.into_parts();

                self.encode_head(parts, &res_body)?;

                let mut encoder = res_body.encoder(self.ctx.ctype);

                match body_handle {
                    Some(body_handle) => {
                        ResponseHandler {
                            res_body: Some(res_body),
                            encoder,
                            body_handle,
                            io: &mut self.io,
                        }
                        .await?
                    }
                    None => {
                        tokio::pin!(res_body);

                        while let Some(bytes) = res_body.as_mut().next().await {
                            let bytes = bytes.unwrap();
                            encoder.encode(&bytes, &mut self.io.write_buf)?;
                        }
                        encoder.encode_eof(&mut self.io.write_buf)?;
                    }
                }
            }

            self.io.drain_write().await?;

            self.io.read().await?;
        }
    }

    fn decode_head(&mut self) -> Result<Option<DecodedHead>, Error> {
        // Do not try when nothing new read.
        if self.io.read_buf.advanced() {
            let buf = self.io.read_buf.buf();

            if let Some((req, decoder)) = self.ctx.decode_head(buf)? {
                let (body_handle, body) = RequestBodyHandle::new_pair(decoder);

                let (parts, _) = req.into_parts();
                let req = Request::from_parts(parts, body);

                return Ok(Some((req, body_handle)));
            }
        }

        Ok(None)
    }

    fn encode_head(&mut self, parts: Parts, body: &ResponseBody<B>) -> Result<(), Error> {
        let size = body.size();
        self.ctx.encode_head(parts, size, &mut self.io.write_buf)?;
        Ok(())
    }

    fn set_read_close(&mut self) {
        self.io.state.set_read_close();
    }

    fn set_write_close(&mut self) {
        self.io.state.set_write_close();
    }
}

pin_project! {
    struct RequestHandler<'a, 'b, Fut, B> {
        #[pin]
        fut: Fut,
        body_handle: Option<&'a mut RequestBodyHandle>,
        io: &'a mut Io<'b>,
        _body: PhantomData<B>
    }
}

impl<Fut, E, B> Future for RequestHandler<'_, '_, Fut, B>
where
    Fut: Future<Output = Result<Response<ResponseBody<B>>, E>>,
    E: ResponseError<Response<ResponseBody<B>>>,
{
    type Output = Result<Response<ResponseBody<B>>, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();

        match this.fut.poll(cx) {
            Poll::Ready(res) => {
                let res = res.unwrap_or_else(ResponseError::response_error);
                Poll::Ready(Ok(res))
            }
            // service call is pending. could be waiting for more read.
            Poll::Pending => {
                let mut new = false;
                let mut done = false;

                if let Some(handle) = this.body_handle.as_deref_mut() {
                    let io = this.io;

                    // TODO: read error here should be treated as partial close.
                    // Which means body_handle should treat error as finished read.
                    // pass the partial buffer to service call and let it decide what to do.
                    ready!(io.io.poll_read_ready(cx))?;
                    io.try_read()?;

                    if io.read_buf.advanced() {
                        let buf = io.read_buf.buf();

                        while let Some(item) = handle.decoder.decode(buf)? {
                            new = true;
                            match item {
                                RequestBodyItem::Chunk(bytes) => handle.sender.feed_data(bytes),
                                RequestBodyItem::Eof => {
                                    handle.sender.feed_eof();
                                    done = true;
                                }
                            }
                        }
                    }
                }

                if done {
                    *this.body_handle = None;
                }

                if new {
                    self.poll(cx)
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

pin_project! {
    struct ResponseHandler<'a, 'b, B> {
        #[pin]
        res_body: Option<ResponseBody<B>>,
        encoder: TransferEncoding,
        body_handle: RequestBodyHandle,
        io: &'a mut Io<'b>,
    }
}

impl<B, E> Future for ResponseHandler<'_, '_, B>
where
    B: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,
{
    type Output = Result<(), Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        let io = this.io;

        while let Some(res_body) = this.res_body.as_mut().as_pin_mut() {
            match res_body.poll_next(cx) {
                Poll::Pending => break,
                Poll::Ready(Some(bytes)) => {
                    let bytes = bytes.unwrap();
                    this.encoder.encode(&bytes, &mut io.write_buf)?;
                }
                Poll::Ready(None) => {
                    this.encoder.encode_eof(&mut io.write_buf)?;
                    this.res_body.set(None);
                }
            }
        }

        if io.io.poll_write_ready(cx)?.is_ready() {
            let _ = io.try_write()?;
        }

        let mut new = false;
        let mut done = false;

        // TODO: read error here should be treated as partial close.
        // Which means body_handle should treat error as finished read.
        // pass the partial buffer to service call and let it decide what to do.
        while io.io.poll_read_ready(cx)?.is_ready() {
            let _ = io.try_read()?;

            if io.read_buf.advanced() {
                let buf = io.read_buf.buf();
                while let Some(item) = this.body_handle.decoder.decode(buf)? {
                    new = true;
                    match item {
                        RequestBodyItem::Chunk(bytes) => this.body_handle.sender.feed_data(bytes),
                        RequestBodyItem::Eof => {
                            this.body_handle.sender.feed_eof();
                            done = true;
                            break;
                        }
                    }
                }
            }
        }

        if new {
            self.poll(cx)
        } else if this.res_body.is_none() && done {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

type DecodedHead = (Request<RequestBody>, Option<RequestBodyHandle>);

struct ReadBuffer {
    advanced: bool,
    buf: BytesMut,
}

impl ReadBuffer {
    fn new() -> Self {
        Self {
            advanced: false,
            buf: BytesMut::new(),
        }
    }

    fn buf(&mut self) -> &mut BytesMut {
        &mut self.buf
    }

    #[inline(always)]
    fn advanced(&self) -> bool {
        self.advanced
    }

    #[inline(always)]
    fn advance(&mut self, advanced: bool) {
        self.advanced = advanced;
    }
}

struct RequestBodyHandle {
    decoder: RequestBodyDecoder,
    sender: RequestBodySender,
}

impl RequestBodyHandle {
    fn new_pair(decoder: RequestBodyDecoder) -> (Option<RequestBodyHandle>, RequestBody) {
        if decoder.is_eof() {
            let (_, body) = RequestBody::create(true);

            (None, body)
        } else {
            let (sender, body) = RequestBody::create(false);

            let body_handle = RequestBodyHandle { decoder, sender };

            (Some(body_handle), body)
        }
    }

    fn close(&mut self) {
        self.sender.feed_eof();
    }
}
