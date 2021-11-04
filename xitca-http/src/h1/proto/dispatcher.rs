use std::{
    future::Future,
    io,
    marker::PhantomData,
    pin::Pin,
    task::{self, Poll},
    time::Duration,
};

use futures_core::{ready, stream::Stream};
use http::{response::Parts, Request, Response};
use tokio::{
    io::{AsyncWrite, Interest},
    pin,
};
use tracing::trace;
use xitca_io::io::{AsyncIo, Ready};
use xitca_service::Service;

use crate::{
    body::ResponseBody,
    bytes::Bytes,
    config::HttpServiceConfig,
    date::DateTime,
    error::BodyError,
    flow::HttpFlowInner,
    h1::{
        body::{RequestBody, RequestBodySender},
        error::Error,
    },
    response,
    util::{
        futures::{never, poll_fn, Select, SelectOutput, Timeout},
        hint::unlikely,
        keep_alive::KeepAlive,
    },
};

use super::{
    buf::{FlatBuf, ListBuf, WriteBuf},
    codec::TransferCoding,
    context::{ConnectionType, Context},
    error::{Parse, ProtoError},
};

/// function to generic over different writer buffer types dispatcher.
pub(crate) async fn run<
    'a,
    St,
    S,
    ReqB,
    ResB,
    E,
    X,
    U,
    D,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
>(
    io: &'a mut St,
    timer: Pin<&'a mut KeepAlive>,
    config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    flow: &'a HttpFlowInner<S, X, U>,
    date: &'a D,
) -> Result<(), Error<S::Error>>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<ResB>>> + 'static,

    X: Service<Request<ReqB>, Response = Request<ReqB>> + 'static,

    ReqB: From<RequestBody>,

    ResB: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,

    S::Error: From<X::Error>,

    St: AsyncIo,

    D: DateTime,
{
    let is_vectored = if config.force_flat_buf {
        false
    } else {
        io.is_write_vectored()
    };

    let res = if is_vectored {
        let write_buf = ListBuf::<_, WRITE_BUF_LIMIT>::default();
        Dispatcher::new(io, timer, config, flow, date, write_buf).run().await
    } else {
        let write_buf = FlatBuf::<WRITE_BUF_LIMIT>::default();
        Dispatcher::new(io, timer, config, flow, date, write_buf).run().await
    };

    match res {
        Ok(_) | Err(Error::Closed) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Http/1 dispatcher
struct Dispatcher<
    'a,
    St,
    S,
    ReqB,
    X,
    U,
    W,
    D,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> where
    S: Service<Request<ReqB>>,
{
    io: Io<'a, St, W, S::Error, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    timer: Pin<&'a mut KeepAlive>,
    ka_dur: Duration,
    ctx: Context<'a, D, HEADER_LIMIT>,
    flow: &'a HttpFlowInner<S, X, U>,
    _phantom: PhantomData<ReqB>,
}

struct Io<'a, St, W, E, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> {
    io: &'a mut St,
    read_buf: FlatBuf<READ_BUF_LIMIT>,
    write_buf: W,
    _err: PhantomData<E>,
}

impl<'a, St, W, E, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Io<'a, St, W, E, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    St: AsyncIo,
    W: WriteBuf,
{
    fn new(io: &'a mut St, write_buf: W) -> Self {
        Self {
            io,
            read_buf: FlatBuf::new(),
            write_buf,
            _err: PhantomData,
        }
    }

    /// read until blocked/read backpressure and advance readbuf.
    fn try_read(&mut self) -> Result<(), Error<E>> {
        loop {
            match self.io.try_read_buf(&mut *self.read_buf) {
                Ok(0) => return Err(Error::Closed),
                Ok(_) => {
                    if self.read_buf.backpressure() {
                        trace!(target: "h1_dispatcher", "Read buffer limit reached(Current length: {} bytes). Entering backpressure(No log event for recovery).", self.read_buf.len());
                        return Ok(());
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Return when write is blocked and need wait.
    fn try_write(&mut self) -> Result<(), Error<E>> {
        self.write_buf.try_write_io(self.io)
    }

    /// Block task and read.
    async fn read(&mut self) -> Result<(), Error<E>> {
        let _ = self.io.ready(Interest::READABLE).await?;
        self.try_read()
    }

    /// Flush io
    async fn flush(&mut self) -> Result<(), Error<E>> {
        poll_fn(|cx| Pin::new(&mut *self.io).poll_flush(cx))
            .await
            .map_err(Error::from)
    }

    /// drain write buffer and flush the io.
    async fn drain_write(&mut self) -> Result<(), Error<E>> {
        while !self.write_buf.is_empty() {
            self.try_write()?;
            let _ = self.io.ready(Interest::WRITABLE).await?;
        }
        self.flush().await
    }

    /// A specialized readable check that always pending when read buffer is full.
    /// This is a hack for `crate::util::futures::Select`.
    async fn readable<D, const HEADER_LIMIT: usize>(
        &self,
        handle: &mut RequestBodyHandle,
        ctx: &mut Context<'_, D, HEADER_LIMIT>,
    ) -> io::Result<()> {
        if self.read_buf.backpressure() {
            never().await
        } else {
            let _ = self.io.ready(Interest::READABLE).await?;
            // Check the readiness of RequestBodyHandle
            // so read ahead does not buffer too much data.
            handle.ready(ctx).await
        }
    }

    /// A specialized writable check that always pending when write buffer is empty.
    /// This is a hack for `crate::util::futures::Select`.
    async fn writable(&self) -> Result<(), Error<E>> {
        if self.write_buf.is_empty() {
            never().await
        } else {
            let _ = self.io.ready(Interest::WRITABLE).await?;
            Ok(())
        }
    }

    /// Check readable and writable state of IO and ready state of request payload sender.
    /// Remove redable state if request payload is not ready(Read backpressure).
    async fn ready<D, const HEADER_LIMIT: usize>(
        &self,
        handle: &mut RequestBodyHandle,
        ctx: &mut Context<'_, D, HEADER_LIMIT>,
    ) -> io::Result<Ready> {
        struct ReadyFuture<'a, 'c, F, D, const HEADER_LIMIT: usize> {
            fut: Pin<&'a mut F>,
            handle: &'a RequestBodyHandle,
            ctx: &'a mut Context<'c, D, HEADER_LIMIT>,
        }

        impl<F, D, const HEADER_LIMIT: usize> Future for ReadyFuture<'_, '_, F, D, HEADER_LIMIT>
        where
            F: Future<Output = io::Result<Ready>>,
        {
            type Output = io::Result<Ready>;

            fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
                let this = self.get_mut();
                let ready = ready!(this.fut.as_mut().poll(cx))?;
                match this.handle.sender.poll_ready(cx) {
                    Poll::Ready(Ok(_)) => Poll::Ready(Ok(ready)),
                    Poll::Ready(Err(e)) => {
                        // When service call dropped payload there is no tell how many bytes
                        // still remain readable in the connection.
                        // close the connection would be a safe bet than draining it.
                        this.ctx.set_force_close_on_error();
                        Poll::Ready(Err(e))
                    }
                    // on pending path the readable ready state should be removed.
                    // It indicate the read ahead buffer is at full capacity.
                    Poll::Pending => Poll::Ready(Ok(ready - Ready::READABLE)),
                }
            }
        }

        let interest = match (self.read_buf.backpressure(), self.write_buf.is_empty()) {
            (true, true) => return never().await,
            (false, true) => Interest::READABLE,
            (true, false) => Interest::WRITABLE,
            (false, false) => Interest::READABLE | Interest::WRITABLE,
        };

        let fut = self.io.ready(interest);
        pin!(fut);

        ReadyFuture { fut, handle, ctx }.await
    }

    #[inline(never)]
    async fn shutdown(&mut self) -> Result<(), Error<E>> {
        self.drain_write().await?;
        self.flush().await
    }
}

impl<
        'a,
        St,
        S,
        ReqB,
        ResB,
        E,
        X,
        U,
        W,
        D,
        const HEADER_LIMIT: usize,
        const READ_BUF_LIMIT: usize,
        const WRITE_BUF_LIMIT: usize,
    > Dispatcher<'a, St, S, ReqB, X, U, W, D, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<ResB>>> + 'static,

    X: Service<Request<ReqB>, Response = Request<ReqB>> + 'static,

    ReqB: From<RequestBody>,

    ResB: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,

    S::Error: From<X::Error>,

    St: AsyncIo,
    W: WriteBuf,

    D: DateTime,
{
    fn new(
        io: &'a mut St,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
        flow: &'a HttpFlowInner<S, X, U>,
        date: &'a D,
        write_buf: W,
    ) -> Self {
        Self {
            io: Io::new(io, write_buf),
            timer,
            ka_dur: config.keep_alive_timeout,
            ctx: Context::new(date),
            flow,
            _phantom: PhantomData,
        }
    }

    async fn run(mut self) -> Result<(), Error<S::Error>> {
        loop {
            match self.ctx.ctype() {
                ConnectionType::Init | ConnectionType::KeepAlive => {
                    match self.io.read().timeout(self.timer.as_mut()).await {
                        Ok(res) => res?,
                        Err(_) => {
                            trace!(target: "h1_dispatcher", "Connection timeout. Shutting down");
                            return self.io.shutdown().await;
                        }
                    }
                }
                ConnectionType::Upgrade | ConnectionType::Close => {
                    trace!(target: "h1_dispatcher", "Connection not keep-alive. Shutting down");
                    return self.io.shutdown().await;
                }
                ConnectionType::CloseForce => {
                    unlikely();
                    trace!(target: "h1_dispatcher", "Connection meets force close condition. Closing");
                    return Ok(());
                }
            }

            'req: while let Some(res) = self.decode_head() {
                match res {
                    Ok((req, mut body_handle)) => {
                        // have new request. update timer deadline.
                        let now = self.ctx.date.now() + self.ka_dur;
                        self.timer.as_mut().update(now);

                        let (parts, res_body) = self.request_handler(req, &mut body_handle).await?.into_parts();

                        let encoder = &mut self.encode_head(parts, &res_body)?;

                        self.response_handler(res_body, encoder, body_handle).await?;

                        if self.ctx.is_connection_closed() {
                            break 'req;
                        }
                    }
                    Err(ProtoError::Parse(Parse::HeaderTooLarge)) => {
                        self.request_error(response::header_too_large)?;
                        break 'req;
                    }
                    Err(ProtoError::Parse(_)) => {
                        self.request_error(response::bad_request)?;
                        break 'req;
                    }
                    // TODO: handle error that are meant to be a response.
                    Err(e) => return Err(e.into()),
                };
            }

            self.io.drain_write().await?;
        }
    }

    fn decode_head(&mut self) -> Option<Result<DecodedHead<ReqB>, ProtoError>> {
        match self.ctx.decode_head::<READ_BUF_LIMIT>(&mut *self.io.read_buf) {
            Ok(Some((req, decoder))) => {
                let (body_handle, body) = RequestBodyHandle::new_pair(decoder);

                let (parts, _) = req.into_parts();
                let req = Request::from_parts(parts, body);

                Some(Ok((req, body_handle)))
            }
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }

    fn encode_head(&mut self, parts: Parts, body: &ResponseBody<ResB>) -> Result<TransferCoding, Error<S::Error>> {
        self.ctx
            .encode_head(parts, body.size(), &mut self.io.write_buf)
            .map_err(Error::from)
    }

    async fn request_handler(
        &mut self,
        mut req: Request<ReqB>,
        body_handle: &mut Option<RequestBodyHandle>,
    ) -> Result<S::Response, Error<S::Error>> {
        if self.ctx.is_expect_header() {
            match self.flow.expect.call(req).await {
                Ok(expect_res) => {
                    // encode continue
                    self.ctx.encode_continue(&mut self.io.write_buf);

                    // use drain write to make sure continue is sent to client.
                    // the world can wait until it happens.
                    self.io.drain_write().await?;

                    req = expect_res;
                }
                Err(e) => return Err(Error::Service(e.into())),
            }
        };

        let fut = self.flow.service.call(req);

        pin!(fut);

        match fut.as_mut().select(self.request_body_handler(body_handle)).await {
            SelectOutput::A(res) => res.map_err(Error::Service),
            SelectOutput::B(res) => res,
        }
    }

    async fn request_body_handler(
        &mut self,
        body_handle: &mut Option<RequestBodyHandle>,
    ) -> Result<S::Response, Error<S::Error>> {
        // decode request body and read more if needed.
        while let Some(handle) = body_handle {
            match handle.decode(&mut self.io.read_buf)? {
                DecodeState::Continue => {
                    self.io.readable(handle, &mut self.ctx).await?;
                    self.io.try_read()?;
                }
                DecodeState::Eof => *body_handle = None,
            }
        }

        // pending and do nothing when RequestBodyHandle is gone.
        never().await
    }

    async fn response_handler(
        &mut self,
        body: ResponseBody<ResB>,
        encoder: &mut TransferCoding,
        mut body_handle: Option<RequestBodyHandle>,
    ) -> Result<(), Error<S::Error>> {
        pin!(body);

        loop {
            if self.io.write_buf.backpressure() {
                trace!(target: "h1_dispatcher", "Write buffer limit reached. Enter backpressure.");
                self.io.drain_write().await?;
                trace!(target: "h1_dispatcher", "Write buffer empty. Recover from backpressure.");
            } else if let Some(handle) = body_handle.as_mut() {
                match handle.decode(&mut self.io.read_buf)? {
                    DecodeState::Continue => {
                        match body.as_mut().next().select(self.io.ready(handle, &mut self.ctx)).await {
                            SelectOutput::A(Some(bytes)) => {
                                let bytes = bytes?;
                                encoder.encode(bytes, &mut self.io.write_buf);
                            }
                            SelectOutput::A(None) => {
                                // Request body is partial consumed.
                                // Close connection in case there are bytes remain in socket.
                                if !handle.sender.is_eof() {
                                    self.ctx.set_force_close_on_non_eof();
                                };

                                encoder.encode_eof(&mut self.io.write_buf);

                                return Ok(());
                            }
                            SelectOutput::B(Ok(ready)) => {
                                if ready.is_readable() {
                                    self.io.try_read()?
                                }
                                if ready.is_writable() {
                                    self.io.try_write()?;
                                    self.io.flush().await?;
                                }
                            }
                            // TODO: potential special handling error case of RequestBodySender::poll_ready ?
                            // This output would mix IO error and RequestBodySender::poll_ready error.
                            // For now the effect of not handling them differently is not clear.
                            SelectOutput::B(Err(e)) => {
                                handle.sender.feed_error(e.into());
                                body_handle = None;
                            }
                        }
                    }
                    DecodeState::Eof => body_handle = None,
                }
            } else {
                match body.as_mut().next().select(self.io.writable()).await {
                    SelectOutput::A(Some(bytes)) => {
                        let bytes = bytes?;
                        encoder.encode(bytes, &mut self.io.write_buf);
                    }
                    SelectOutput::A(None) => {
                        encoder.encode_eof(&mut self.io.write_buf);
                        return Ok(());
                    }
                    SelectOutput::B(res) => {
                        res?;
                        self.io.try_write()?;
                        self.io.flush().await?;
                    }
                }
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn request_error<F>(&mut self, func: F) -> Result<(), Error<S::Error>>
    where
        F: Fn() -> Response<ResponseBody<ResB>>,
    {
        // Header is too large to be parsed.
        // Close the connection after sending error response as it's pointless
        // to read the remaining bytes inside connection.
        self.ctx.set_force_close_on_error();

        let (parts, res_body) = func().into_parts();

        self.encode_head(parts, &res_body).map(|_| ())
    }
}

type DecodedHead<ReqB> = (Request<ReqB>, Option<RequestBodyHandle>);

struct RequestBodyHandle {
    decoder: TransferCoding,
    sender: RequestBodySender,
}

enum DecodeState {
    /// TransferDecoding can continue for more data.
    Continue,
    /// TransferDecoding is ended with eof.
    Eof,
}

impl RequestBodyHandle {
    fn new_pair<ReqB>(decoder: TransferCoding) -> (Option<Self>, ReqB)
    where
        ReqB: From<RequestBody>,
    {
        if decoder.is_eof() {
            let body = RequestBody::empty();
            (None, body.into())
        } else {
            let (sender, body) = RequestBody::create(false);
            let body_handle = RequestBodyHandle { decoder, sender };
            (Some(body_handle), body.into())
        }
    }

    fn decode<const READ_BUF_LIMIT: usize>(
        &mut self,
        read_buf: &mut FlatBuf<READ_BUF_LIMIT>,
    ) -> io::Result<DecodeState> {
        while let Some(bytes) = self.decoder.decode(&mut *read_buf)? {
            if bytes.is_empty() {
                self.sender.feed_eof();
                return Ok(DecodeState::Eof);
            } else {
                self.sender.feed_data(bytes);
            }
        }

        Ok(DecodeState::Continue)
    }

    async fn ready<D, const HEADER_LIMIT: usize>(&self, ctx: &mut Context<'_, D, HEADER_LIMIT>) -> io::Result<()> {
        self.sender.ready().await.map_err(move |e| {
            // When service call dropped payload there is no tell how many bytes
            // still remain readable in the connection.
            // close the connection would be a safe bet than draining it.
            ctx.set_force_close_on_error();
            e
        })
    }
}
