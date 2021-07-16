use std::{io, marker::PhantomData, pin::Pin, time::Duration};

use bytes::Bytes;
use futures_core::stream::Stream;
use http::{response::Parts, Request, Response};
use tokio::{
    io::{AsyncWrite, Interest},
    pin,
};
use tracing::trace;
use xitca_server::net::AsyncReadWrite;
use xitca_service::Service;

use crate::body::ResponseBody;
use crate::config::HttpServiceConfig;
use crate::error::BodyError;
use crate::flow::HttpFlowInner;
use crate::h1::{
    body::{RequestBody, RequestBodySender},
    error::Error,
};
use crate::response;
use crate::util::{
    date::Date,
    futures::{never, poll_fn, Select, SelectOutput, Timeout},
    hint::unlikely,
    keep_alive::KeepAlive,
};

use super::buf::{FlatWriteBuf, ListWriteBuf, ReadBuf, WriteBuf};
use super::context::{ConnectionType, Context};
use super::decode::TransferDecoding;
use super::encode::TransferEncoding;
use super::error::{Parse, ProtoError};

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
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
>(
    io: &'a mut St,
    timer: Pin<&'a mut KeepAlive>,
    config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    flow: &'a HttpFlowInner<S, X, U>,
    date: &'a Date,
) -> Result<(), Error<S::Error>>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<ResB>>> + 'static,

    X: Service<Request<ReqB>, Response = Request<ReqB>> + 'static,

    ReqB: From<RequestBody>,

    ResB: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,

    S::Error: From<X::Error>,

    St: AsyncReadWrite,
{
    let is_vectored = if config.force_flat_buf {
        false
    } else {
        io.is_write_vectored()
    };

    let res = if is_vectored {
        let write_buf = ListWriteBuf::default();
        Dispatcher::new(io, timer, config, flow, date, write_buf).run().await
    } else {
        let write_buf = FlatWriteBuf::default();
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
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> where
    S: Service<Request<ReqB>>,
{
    io: Io<'a, St, W, S::Error, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    timer: Pin<&'a mut KeepAlive>,
    ka_dur: Duration,
    ctx: Context<'a, HEADER_LIMIT>,
    flow: &'a HttpFlowInner<S, X, U>,
    _phantom: PhantomData<ReqB>,
}

struct Io<'a, St, W, E, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> {
    io: &'a mut St,
    read_buf: ReadBuf<READ_BUF_LIMIT>,
    write_buf: W,
    _err: PhantomData<E>,
}

impl<'a, St, W, E, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Io<'a, St, W, E, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    St: AsyncReadWrite,
    W: WriteBuf<WRITE_BUF_LIMIT>,
{
    fn new(io: &'a mut St, write_buf: W) -> Self {
        Self {
            io,
            read_buf: ReadBuf::new(),
            write_buf,
            _err: PhantomData,
        }
    }

    /// read until blocked/read backpressure and advance readbuf.
    fn try_read(&mut self) -> Result<(), Error<E>> {
        let buf = &mut self.read_buf;

        loop {
            match self.io.try_read_buf(buf.buf_mut()) {
                Ok(0) => return Err(Error::Closed),
                Ok(_) => {
                    if buf.backpressure() {
                        trace!(target: "h1_dispatcher", "Read buffer limit reached(Current length: {} bytes). Entering backpressure(No log event for recovery).", buf.len());
                        return Ok(());
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Return true when write is blocked and need wait.
    /// Return false when write is finished.(Did not blocked)
    fn try_write(&mut self) -> Result<bool, Error<E>> {
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
        while self.try_write()? {
            let _ = self.io.ready(Interest::WRITABLE).await?;
        }
        self.flush().await
    }

    /// A specialized readable check that always pending when read buffer is full.
    /// This is a hack for tokio::select macro.
    async fn readable(&self) -> Result<(), Error<E>> {
        if self.read_buf.backpressure() {
            never().await
        } else {
            let _ = self.io.ready(Interest::READABLE).await?;
            Ok(())
        }
    }

    /// A specialized writable check that always pending when write buffer is empty.
    /// This is a hack for tokio::select macro.
    async fn writable(&self) -> Result<(), Error<E>> {
        if self.write_buf.empty() {
            never().await
        } else {
            let _ = self.io.ready(Interest::WRITABLE).await?;
            Ok(())
        }
    }

    /// Return Ok when read is done or body handle is dropped by service call.
    async fn handle_request_body<const HEADER_LIMIT: usize>(
        &mut self,
        handle: &mut RequestBodyHandle,
        ctx: &mut Context<'_, HEADER_LIMIT>,
    ) -> Result<(), Error<E>> {
        while poll_fn(|cx| handle.sender.poll_ready(cx)).await.is_ok() {
            self.readable().await?;
            self.try_read()?;

            while let Some(bytes) = handle.decoder.decode(self.read_buf.buf_mut())? {
                if bytes.is_empty() {
                    handle.sender.feed_eof();
                    return Ok(());
                } else {
                    handle.sender.feed_data(bytes)
                }
            }
        }

        // When service call dropped payload there is no tell how many bytes
        // still remain readable in the connection.
        // close the connection would be a safe bet than draining it.
        ctx.set_force_close();

        Ok(())
    }

    /// A ready check version of `Io::handle_request_body`.
    /// This is to avoid borrow self as mut to co-op well with `tokio::select` macro.
    async fn request_body_ready<const HEADER_LIMIT: usize>(
        &self,
        body_handle: &mut Option<RequestBodyHandle>,
        ctx: &mut Context<'_, HEADER_LIMIT>,
    ) -> Result<RequestBodyHandle, Error<E>> {
        if let Some(handle) = body_handle.as_mut() {
            match poll_fn(|cx| handle.sender.poll_ready(cx)).await {
                Ok(_) => {
                    self.readable().await?;
                    // TODO: This is an unwrap happen with every successful check. get rid of it.
                    return Ok(body_handle.take().unwrap());
                }
                Err(_) => {
                    // When service call dropped payload there is no tell how many bytes
                    // still remain readable in the connection.
                    // close the connection would be a safe bet than draining it.
                    ctx.set_force_close();
                    *body_handle = None;
                }
            }
        }

        // A future that would never resolve. This is a hack to work with tokio::select macro.
        never().await
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
        const HEADER_LIMIT: usize,
        const READ_BUF_LIMIT: usize,
        const WRITE_BUF_LIMIT: usize,
    > Dispatcher<'a, St, S, ReqB, X, U, W, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<ReqB>, Response = Response<ResponseBody<ResB>>> + 'static,

    X: Service<Request<ReqB>, Response = Request<ReqB>> + 'static,

    ReqB: From<RequestBody>,

    ResB: Stream<Item = Result<Bytes, E>>,
    BodyError: From<E>,

    S::Error: From<X::Error>,

    St: AsyncReadWrite,
    W: WriteBuf<WRITE_BUF_LIMIT>,
{
    fn new(
        io: &'a mut St,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
        flow: &'a HttpFlowInner<S, X, U>,
        date: &'a Date,
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
                ConnectionType::Init => {
                    if self.ctx.is_force_close() {
                        unlikely();
                        trace!(target: "h1_dispatcher", "Connection error. Shutting down");
                        return Ok(());
                    } else {
                        // use timer to detect slow connection.
                        match self.io.read().timeout(self.timer.as_mut()).await {
                            Ok(res) => res?,
                            Err(_) => {
                                trace!(target: "h1_dispatcher", "Slow Connection detected. Shutting down");
                                return Ok(());
                            }
                        }
                    }
                }
                ConnectionType::KeepAlive => {
                    if self.ctx.is_force_close() {
                        unlikely();
                        trace!(target: "h1_dispatcher", "Connection is keep-alive but meet a force close condition. Shutting down");
                        return self.io.shutdown().await;
                    } else {
                        match self.io.read().timeout(self.timer.as_mut()).await {
                            Ok(res) => res?,
                            Err(_) => {
                                trace!(target: "h1_dispatcher", "Connection keep-alive timeout. Shutting down");
                                return self.io.shutdown().await;
                            }
                        }
                    }
                }
                ConnectionType::Upgrade | ConnectionType::Close => {
                    trace!(target: "h1_dispatcher", "Connection not keep-alive. Shutting down");
                    return self.io.shutdown().await;
                }
            }

            'req: while let Some(res) = self.decode_head() {
                match res {
                    Ok((req, mut body_handle)) => {
                        // have new request. update timer deadline.
                        let now = self.ctx.date.borrow().now() + self.ka_dur;
                        self.timer.as_mut().update(now);

                        let (parts, res_body) = self.request_handler(req, &mut body_handle).await?.into_parts();

                        self.encode_head(parts, &res_body)?;

                        let encoder = &mut res_body.encoder(self.ctx.ctype());

                        self.response_handler(res_body, encoder, body_handle).await?;
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
        match self.ctx.decode_head::<READ_BUF_LIMIT>(self.io.read_buf.buf_mut()) {
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

    fn encode_head(&mut self, parts: Parts, body: &ResponseBody<ResB>) -> Result<(), Error<S::Error>> {
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

        if let Some(handle) = body_handle {
            match fut
                .as_mut()
                .select(self.io.handle_request_body(handle, &mut self.ctx))
                .await
            {
                SelectOutput::A(res) => return res.map_err(Error::Service),
                SelectOutput::B(res) => {
                    // request body read is done or dropped. remove body_handle.
                    res?;
                    *body_handle = None;
                }
            }
        }

        fut.await.map_err(Error::Service)
    }

    async fn response_handler(
        &mut self,
        body: ResponseBody<ResB>,
        encoder: &mut TransferEncoding,
        mut body_handle: Option<RequestBodyHandle>,
    ) -> Result<(), Error<S::Error>> {
        pin!(body);

        'res: loop {
            if self.io.write_buf.backpressure() {
                trace!(target: "h1_dispatcher", "Write buffer limit reached. Enter backpressure.");
                self.io.drain_write().await?;
                trace!(target: "h1_dispatcher", "Write buffer empty. Recover from backpressure.");
            } else {
                match body
                    .as_mut()
                    .next()
                    .select(self.io.writable())
                    .select(self.io.request_body_ready(&mut body_handle, &mut self.ctx))
                    .await
                {
                    SelectOutput::A(SelectOutput::A(Some(bytes))) => {
                        let bytes = bytes?;
                        encoder.encode(bytes, &mut self.io.write_buf)?;
                    }
                    SelectOutput::A(SelectOutput::A(None)) => {
                        return encoder.encode_eof(&mut self.io.write_buf).map_err(Error::from);
                    }
                    SelectOutput::A(SelectOutput::B(res)) => {
                        res?;
                        let _ = self.io.try_write()?;
                        self.io.flush().await?;
                    }
                    SelectOutput::B(res) => {
                        let mut handle = res?;
                        self.io.try_read()?;

                        while let Some(bytes) = handle.decoder.decode(self.io.read_buf.buf_mut())? {
                            if bytes.is_empty() {
                                handle.sender.feed_eof();
                                continue 'res;
                            } else {
                                handle.sender.feed_data(bytes);
                            }
                        }

                        body_handle = Some(handle);
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
        self.ctx.set_force_close();

        let (parts, res_body) = func().into_parts();

        self.encode_head(parts, &res_body)
    }
}

type DecodedHead<ReqB> = (Request<ReqB>, Option<RequestBodyHandle>);

struct RequestBodyHandle {
    decoder: TransferDecoding,
    sender: RequestBodySender,
}

impl RequestBodyHandle {
    fn new_pair<ReqB>(decoder: TransferDecoding) -> (Option<Self>, ReqB)
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
}
