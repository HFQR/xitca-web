use std::{
    future::Future,
    io,
    marker::PhantomData,
    pin::Pin,
    task::{self, Poll},
    time::Duration,
};

use actix_server_alt::net::AsyncReadWrite;
use actix_service_alt::Service;
use bytes::{Buf, Bytes};
use futures_core::Stream;
use http::{response::Parts, Request, Response};
use log::trace;
use pin_project_lite::pin_project;
use tokio::{io::Interest, pin, select};

use crate::body::ResponseBody;
use crate::config::HttpServiceConfig;
use crate::error::BodyError;
use crate::flow::HttpFlowInner;
use crate::h1::{
    body::{RequestBody, RequestBodySender, RequestBodyStatus},
    error::Error,
};
use crate::response::ResponseError;
use crate::util::{date::DateTimeTask, keep_alive::KeepAlive, poll_fn::poll_fn};

use super::buf::{ReadBuf, WriteBuf};
use super::context::{ConnectionType, Context};
use super::decode::{RequestBodyDecoder, RequestBodyItem};
use super::encode::TransferEncoding;
use super::error::ProtoError;

/// Http/1 dispatcher
pub(crate) struct Dispatcher<'a, St, S, ReqB, X, U> {
    io: Io<'a, St>,
    timer: Pin<&'a mut KeepAlive>,
    ka_dur: Duration,
    ctx: Context<'a>,
    flow: &'a HttpFlowInner<S, X, U>,
    _phantom: PhantomData<ReqB>,
}

struct Io<'a, St> {
    io: &'a mut St,
    read_buf: ReadBuf,
    write_buf: WriteBuf,
}

impl<St> Io<'_, St>
where
    St: AsyncReadWrite,
{
    /// read until blocked and advance readbuf.
    fn try_read(&mut self) -> Result<(), Error> {
        let read_buf = &mut self.read_buf;
        read_buf.advance(false);

        loop {
            match self.io.try_read_buf(read_buf.buf_mut()) {
                Ok(0) => return Err(Error::Closed),
                Ok(_) => read_buf.advance(true),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Return true when write is blocked and need wait.
    /// Return false when write is finished.(Did not blocked)
    fn try_write(&mut self) -> Result<bool, Error> {
        match self.write_buf {
            WriteBuf::List(ref mut list) => {
                let queue = list.list_mut();
                while queue.remaining() > 0 {
                    let mut iovs = [io::IoSlice::new(&[]); 64];
                    let len = queue.chunks_vectored(&mut iovs);
                    match self.io.try_write_vectored(&iovs[..len]) {
                        Ok(0) => return Err(Error::Closed),
                        Ok(n) => queue.advance(n),
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                            return Ok(true);
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            WriteBuf::Flat(ref mut buf) => {
                let mut written = 0;
                let len = buf.len();

                while written < len {
                    match self.io.try_write(&buf[written..]) {
                        Ok(0) => return Err(Error::Closed),
                        Ok(n) => written += n,
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                            buf.advance(written);
                            return Ok(true);
                        }
                        Err(e) => return Err(e.into()),
                    }
                }

                buf.clear();
            }
        }

        Ok(false)
    }

    /// Block task and read.
    #[inline(always)]
    async fn read(&mut self) -> Result<(), Error> {
        let _ = self.io.ready(Interest::READABLE).await?;
        self.try_read()
    }

    /// drain write buffer and flush the io.
    #[inline(always)]
    async fn drain_write(&mut self) -> Result<(), Error> {
        while self.try_write()? {
            let _ = self.io.ready(Interest::WRITABLE).await?;
        }
        poll_fn(|cx| Pin::new(&mut *self.io).poll_flush(cx)).await?;
        Ok(())
    }

    /// Return true when new data is decoded.
    fn poll_read_decode_body(
        &mut self,
        body_handle: &mut Option<RequestBodyHandle>,
        ctx: &mut Context<'_>,
        cx: &mut task::Context<'_>,
    ) -> Result<bool, Error> {
        match *body_handle {
            Some(ref mut handle) => match handle.sender.need_read(cx) {
                RequestBodyStatus::Read => {
                    let mut new = false;
                    let mut done = false;

                    // TODO: read error here should be treated as partial close.
                    // Which means body_handle should treat error as finished read.
                    // pass the partial buffer to service call and let it decide what to do.
                    'read: while self.io.poll_read_ready(cx)?.is_ready() {
                        let _ = self.try_read()?;

                        let buf = &mut self.read_buf;

                        if buf.advanced() {
                            while let Some(item) = handle.decoder.decode(buf.buf_mut())? {
                                new = true;
                                match item {
                                    RequestBodyItem::Chunk(bytes) => handle.sender.feed_data(bytes),
                                    RequestBodyItem::Eof => {
                                        handle.sender.feed_eof();
                                        done = true;
                                        break 'read;
                                    }
                                }
                            }
                        }
                    }

                    // remove body handle when client sent eof chunk.
                    // No more read is needed.
                    if done {
                        *body_handle = None;
                    }

                    Ok(new)
                }
                RequestBodyStatus::Pause => Ok(false),
                RequestBodyStatus::Dropped => {
                    *body_handle = None;
                    // When service call dropped payload there is no tell how many bytes still
                    // remain readable in the connection.
                    // close the connection would be a safe bet than draining it.
                    ctx.set_force_close();
                    Ok(false)
                }
            },
            None => Ok(false),
        }
    }
}

impl<'a, St, S, ReqB, ResB, E, X, U> Dispatcher<'a, St, S, ReqB, X, U>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<ResB>>> + 'static,
    S::Error: ResponseError<S::Response>,

    X: Service<Request<ReqB>, Response = Request<ReqB>> + 'static,
    X::Error: ResponseError<S::Response>,

    ReqB: From<RequestBody>,

    ResB: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,

    St: AsyncReadWrite,
{
    pub(crate) fn new(
        io: &'a mut St,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig,
        flow: &'a HttpFlowInner<S, X, U>,
        date: &'a DateTimeTask,
    ) -> Self {
        let is_vectored = if config.http1_pipeline {
            false
        } else {
            io.is_write_vectored()
        };

        let io = Io {
            io,
            read_buf: ReadBuf::new(),
            write_buf: WriteBuf::new(is_vectored),
        };

        Self {
            io,
            timer,
            ka_dur: config.keep_alive_timeout,
            ctx: Context::new(date.get()),
            flow,
            _phantom: PhantomData,
        }
    }

    fn decode_head(&mut self) -> Option<Result<DecodedHead<ReqB>, ProtoError>> {
        // Do not try when nothing new read.
        if self.io.read_buf.advanced() {
            let buf = self.io.read_buf.buf_mut();

            match self.ctx.decode_head(buf) {
                Ok(Some((req, decoder))) => {
                    let (body_handle, body) = RequestBodyHandle::new_pair(decoder);

                    let (parts, _) = req.into_parts();
                    let req = Request::from_parts(parts, body);

                    return Some(Ok((req, body_handle)));
                }
                Err(e) => return Some(Err(e)),
                _ => {}
            }
        }

        None
    }

    fn encode_head(&mut self, parts: Parts, body: &ResponseBody<ResB>) -> Result<(), Error> {
        let size = body.size();
        self.ctx.encode_head(parts, size, &mut self.io.write_buf)?;
        Ok(())
    }

    pub(crate) async fn run(mut self) -> Result<(), Error> {
        loop {
            while let Some(res) = self.decode_head() {
                let (res, mut body_handle) = match res {
                    Ok((req, mut body_handle)) => {
                        // have new request. update timer deadline.
                        let now = self.ctx.date.get().now() + self.ka_dur;
                        self.timer.as_mut().update(now);
                        let res = self.request_handler(req, &mut body_handle).await?;
                        (res, body_handle)
                    }
                    // TODO: handle error that are meant to be a response.
                    Err(e) => return Err(e.into()),
                };

                let (parts, res_body) = res.into_parts();

                self.encode_head(parts, &res_body)?;

                let encoder = &mut res_body.encoder(self.ctx.ctype());

                // pin response body beforehand. this way res handler can take a break on write
                // backpressure
                pin!(res_body);

                'res: loop {
                    // borrow every state so it can iter.
                    let handler = ResponseHandler {
                        res_body: res_body.as_mut(),
                        encoder,
                        body_handle: &mut body_handle,
                        io: &mut self.io,
                        ctx: &mut self.ctx,
                    };

                    match handler.await? {
                        ResponseHandlerResult::Ok => break 'res,
                        // write buffer grows too big. drain it.
                        ResponseHandlerResult::WriteBackpressure => {
                            self.io.drain_write().await?;
                        }
                    }
                }
            }

            self.io.drain_write().await?;

            match self.ctx.ctype() {
                ConnectionType::Init => {
                    // use timer to detect slow connection.
                    select! {
                        biased;
                        res = self.io.read() => res?,
                        _ = self.timer.as_mut() => {
                            trace!("Slow Connection detected. Shutting down");
                            return Ok(())
                        }
                    }
                }
                ConnectionType::KeepAlive => {
                    if self.ctx.is_force_close() {
                        trace!("Connection is keep-alive but meet a force close condition. Shutting down");
                        return Ok(());
                    } else {
                        select! {
                            biased;
                            res = self.io.read() => res?,
                            _ = self.timer.as_mut() => {
                                trace!("Connection keep-alive timeout. Shutting down");
                                return Ok(());
                            }
                        }
                    }
                }
                ConnectionType::Upgrade | ConnectionType::Close => {
                    trace!("Connection not keep-alive. Shutting down");
                    return Ok(());
                }
            }
        }
    }

    async fn request_handler(
        &mut self,
        mut req: Request<ReqB>,
        body_handle: &mut Option<RequestBodyHandle>,
    ) -> Result<Response<ResponseBody<ResB>>, Error> {
        if self.ctx.is_expect() {
            match self.flow.expect.call(req).await {
                Ok(expect_res) => {
                    // encode continue
                    self.ctx.encode_continue(&mut self.io.write_buf);

                    // use drain write to make sure continue is sent to client.
                    // the world can wait until it happens.
                    self.io.drain_write().await?;

                    req = expect_res;
                }
                Err(e) => return Ok(ResponseError::response_error(e)),
            }
        };

        RequestHandler {
            fut: self.flow.service.call(req),
            body_handle,
            io: &mut self.io,
            ctx: &mut self.ctx,
        }
        .await
    }
}

pin_project! {
    struct RequestHandler<'a, 'b, St, Fut> {
        #[pin]
        fut: Fut,
        body_handle: &'a mut Option<RequestBodyHandle>,
        io: &'a mut Io<'b, St>,
        ctx: &'a mut Context<'b>,
    }
}

impl<St, Fut, E, ResB> Future for RequestHandler<'_, '_, St, Fut>
where
    Fut: Future<Output = Result<Response<ResponseBody<ResB>>, E>>,
    E: ResponseError<Response<ResponseBody<ResB>>>,

    St: AsyncReadWrite,
{
    type Output = Result<Response<ResponseBody<ResB>>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            match this.fut.as_mut().poll(cx) {
                Poll::Ready(res) => {
                    let res = res.unwrap_or_else(ResponseError::response_error);
                    return Poll::Ready(Ok(res));
                }
                // service call is pending. could be waiting for more read.
                Poll::Pending => {
                    if !this.io.poll_read_decode_body(this.body_handle, this.ctx, cx)? {
                        return Poll::Pending;
                    }
                }
            }
        }
    }
}

struct ResponseHandler<'a, 'b, St, ResB> {
    res_body: Pin<&'a mut ResponseBody<ResB>>,
    encoder: &'a mut TransferEncoding,
    body_handle: &'a mut Option<RequestBodyHandle>,
    io: &'a mut Io<'b, St>,
    ctx: &'a mut Context<'b>,
}

enum ResponseHandlerResult {
    Ok,
    WriteBackpressure,
}

impl<St, ResB, E> Future for ResponseHandler<'_, '_, St, ResB>
where
    ResB: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,

    St: AsyncReadWrite,
{
    type Output = Result<ResponseHandlerResult, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        while !this.io.write_buf.backpressure() {
            match this.res_body.as_mut().poll_next(cx) {
                Poll::Ready(Some(bytes)) => {
                    let bytes = bytes?;
                    this.encoder.encode(bytes, &mut this.io.write_buf)?;
                }
                Poll::Ready(None) => {
                    this.encoder.encode_eof(&mut this.io.write_buf)?;
                    return Poll::Ready(Ok(ResponseHandlerResult::Ok));
                }
                // payload sending is pending.
                // it could be waiting for more read from client.
                Poll::Pending => {
                    // write buffer to client so it can feed us new
                    // chunked requests if there is any.
                    if this.io.io.poll_write_ready(cx)?.is_ready() {
                        let _ = this.io.try_write()?;
                    }

                    if !this.io.poll_read_decode_body(this.body_handle, this.ctx, cx)? {
                        return Poll::Pending;
                    }
                }
            }
        }

        Poll::Ready(Ok(ResponseHandlerResult::WriteBackpressure))
    }
}

type DecodedHead<ReqB> = (Request<ReqB>, Option<RequestBodyHandle>);

struct RequestBodyHandle {
    decoder: RequestBodyDecoder,
    sender: RequestBodySender,
}

impl RequestBodyHandle {
    fn new_pair<ReqB>(decoder: RequestBodyDecoder) -> (Option<Self>, ReqB)
    where
        ReqB: From<RequestBody>,
    {
        if decoder.is_eof() {
            let (_, body) = RequestBody::create(true);
            (None, body.into())
        } else {
            let (sender, body) = RequestBody::create(false);
            let body_handle = RequestBodyHandle { decoder, sender };
            (Some(body_handle), body.into())
        }
    }
}
