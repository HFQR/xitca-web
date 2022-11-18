use std::{
    future::{pending, poll_fn, Future},
    io,
    marker::PhantomData,
    ops::DerefMut,
    pin::Pin,
    time::Duration,
};

use futures_core::stream::Stream;
use tracing::trace;
use xitca_io::io::{AsyncIo, Interest, Ready};
use xitca_service::Service;
use xitca_unsafe_collection::{
    bytes::read_buf,
    futures::{Select as _, SelectOutput},
    pin,
};

use crate::{
    body::NoneBody,
    bytes::Bytes,
    config::HttpServiceConfig,
    date::DateTime,
    h1::{
        body::{RequestBody, RequestBodySender},
        error::Error,
    },
    http::{response::Parts, Response},
    request::{RemoteAddr, Request},
    response,
    util::{futures::Timeout, hint::unlikely, keep_alive::KeepAlive},
};

use super::{
    buf::{BufInterest, BufWrite, FlatBuf, ListBuf},
    codec::{ChunkResult, TransferCoding},
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
    BE,
    D,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
>(
    io: &'a mut St,
    addr: RemoteAddr,
    timer: Pin<&'a mut KeepAlive>,
    config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    service: &'a S,
    date: &'a D,
) -> Result<(), Error<S::Error, BE>>
where
    S: Service<Request<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    St: AsyncIo,
    D: DateTime,
{
    let is_vectored = config.vectored_write && io.is_vectored_write();

    let res = if is_vectored {
        let write_buf = ListBuf::<_, WRITE_BUF_LIMIT>::default();
        Dispatcher::new(io, addr, timer, config, service, date, write_buf)
            .run()
            .await
    } else {
        let write_buf = FlatBuf::<WRITE_BUF_LIMIT>::default();
        Dispatcher::new(io, addr, timer, config, service, date, write_buf)
            .run()
            .await
    };

    match res {
        Ok(_) | Err(Error::Closed) => Ok(()),
        Err(Error::KeepAliveExpire) => {
            trace!(target: "h1_dispatcher", "Connection keep-alive expired. Shutting down");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Http/1 dispatcher
struct Dispatcher<
    'a,
    St,
    S,
    ReqB,
    W,
    D,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
> {
    io: BufferedIo<'a, St, W, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    timer: Pin<&'a mut KeepAlive>,
    ka_dur: Duration,
    ctx: Context<'a, D, HEADER_LIMIT>,
    service: &'a S,
    _phantom: PhantomData<ReqB>,
}

struct BufferedIo<'a, St, W, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> {
    io: &'a mut St,
    read_buf: FlatBuf<READ_BUF_LIMIT>,
    write_buf: W,
}

impl<'a, St, W, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    BufferedIo<'a, St, W, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    St: AsyncIo,
    W: BufWrite,
{
    fn new(io: &'a mut St, write_buf: W) -> Self {
        Self {
            io,
            read_buf: FlatBuf::new(),
            write_buf,
        }
    }

    // read until blocked/read backpressure and advance read_buf.
    fn try_read(&mut self) -> io::Result<()> {
        loop {
            match read_buf(self.io, &mut *self.read_buf) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(_) => {
                    if self.read_buf.backpressure() {
                        trace!(target: "h1_dispatcher", "Read buffer limit reached(Current length: {} bytes). Entering backpressure(No log event for recovery).", self.read_buf.len());
                        return Ok(());
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    fn try_write(&mut self) -> io::Result<()> {
        self.write_buf.flush(self.io)
    }

    async fn read(&mut self) -> io::Result<()> {
        self.io.ready(Interest::READABLE).await?;
        self.try_read()
    }

    // drain write buffer and flush the io.
    async fn drain_write(&mut self) -> io::Result<()> {
        while self.write_buf.want_write() {
            self.io.ready(Interest::WRITABLE).await?;
            self.try_write()?;
        }

        loop {
            match io::Write::flush(&mut self.io) {
                Ok(()) => return Ok(()),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            self.io.ready(Interest::WRITABLE).await?;
        }
    }

    // Check readable and writable state of IO and ready state of request body reader.
    // return error when runtime is shutdown.(See AsyncIo::ready for reason).
    async fn ready<D, const HEADER_LIMIT: usize>(
        &mut self,
        body_reader: &mut BodyReader,
        ctx: &mut Context<'_, D, HEADER_LIMIT>,
    ) -> io::Result<Ready> {
        if !self.write_buf.want_write() {
            body_reader.ready(&mut self.read_buf, ctx).await;
            self.io.ready(Interest::READABLE).await
        } else {
            match body_reader
                .ready(&mut self.read_buf, ctx)
                .select(self.io.ready(Interest::WRITABLE))
                .await
            {
                SelectOutput::A(_) => self.io.ready(Interest::READABLE | Interest::WRITABLE).await,
                SelectOutput::B(res) => res,
            }
        }
    }

    #[cold]
    #[inline(never)]
    async fn shutdown(&mut self) -> io::Result<()> {
        poll_fn(|cx| Pin::new(&mut *self.io).poll_shutdown(cx)).await
    }
}

impl<
        'a,
        St,
        S,
        ReqB,
        ResB,
        BE,
        W,
        D,
        const HEADER_LIMIT: usize,
        const READ_BUF_LIMIT: usize,
        const WRITE_BUF_LIMIT: usize,
    > Dispatcher<'a, St, S, ReqB, W, D, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    St: AsyncIo,
    W: BufWrite,
    D: DateTime,
{
    fn new(
        io: &'a mut St,
        addr: RemoteAddr,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
        service: &'a S,
        date: &'a D,
        write_buf: W,
    ) -> Self {
        Self {
            io: BufferedIo::new(io, write_buf),
            timer,
            ka_dur: config.keep_alive_timeout,
            ctx: Context::with_remote_addr(addr, date),
            service,
            _phantom: PhantomData,
        }
    }

    async fn run(mut self) -> Result<(), Error<S::Error, BE>> {
        loop {
            match self.ctx.ctype() {
                ConnectionType::Init => {}
                ConnectionType::KeepAlive => self.update_timer(),
                ConnectionType::Close => {
                    unlikely();
                    return self.io.shutdown().await.map_err(Into::into);
                }
            }

            self.io.read().timeout(self.timer.as_mut()).await??;

            while let Some(res) = self.decode_head() {
                match res {
                    Ok((req, mut body_reader)) => {
                        let (parts, res_body) = self.request_handler(req, &mut body_reader).await?.into_parts();
                        let encoder = &mut self.encode_head(parts, &res_body)?;
                        self.response_handler(res_body, encoder, &mut body_reader).await?;
                        if self.ctx.is_connection_closed() {
                            break;
                        }
                    }
                    Err(ProtoError::Parse(Parse::HeaderTooLarge)) => {
                        self.request_error(response::header_too_large)?;
                        break;
                    }
                    Err(ProtoError::Parse(_)) => {
                        self.request_error(response::bad_request)?;
                        break;
                    }
                    // TODO: handle error that are meant to be a response.
                    Err(e) => return Err(e.into()),
                };
            }

            // TODO: add timeout for drain write?
            self.io.drain_write().await?;
        }
    }

    // update timer deadline according to keep alive duration.
    fn update_timer(&mut self) {
        let now = self.ctx.date.now() + self.ka_dur;
        self.timer.as_mut().update(now);
    }

    fn decode_head(&mut self) -> Option<Result<DecodedHead<ReqB>, ProtoError>> {
        match self.ctx.decode_head::<READ_BUF_LIMIT>(self.io.read_buf.deref_mut()) {
            Ok(Some((req, decoder))) => {
                let (body_reader, body) = BodyReader::from_coding(decoder);
                let req = req.map_body(move |_| ReqB::from(body));
                Some(Ok((req, body_reader)))
            }
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }

    fn encode_head<B>(&mut self, parts: Parts, body: &B) -> Result<TransferCoding, Error<S::Error, BE>>
    where
        B: Stream,
    {
        self.ctx
            .encode_head(parts, body, &mut self.io.write_buf)
            .map_err(Into::into)
    }

    async fn request_handler(
        &mut self,
        req: Request<ReqB>,
        body_reader: &mut BodyReader,
    ) -> Result<S::Response, Error<S::Error, BE>> {
        match self
            .service
            .call(req)
            .select(self.request_body_handler(body_reader))
            .await
        {
            SelectOutput::A(res) => Result::map_err(res, Error::Service),
            SelectOutput::B(res) => res,
        }
    }

    // a hacky method that output S::Response as Ok part but never actually produce the value.
    async fn request_body_handler(&mut self, body_reader: &mut BodyReader) -> Result<S::Response, Error<S::Error, BE>> {
        if self.ctx.is_expect_header() {
            // Wait for body polled for expect header request.
            body_reader.wait_for_poll().await;
            // encode continue
            self.ctx.encode_continue(&mut self.io.write_buf);
            // use drain write to make sure continue is sent to client.
            self.io.drain_write().await?;
        }

        loop {
            body_reader.ready(&mut self.io.read_buf, &mut self.ctx).await;
            self.io.read().await?;
        }
    }

    async fn response_handler(
        &mut self,
        body: ResB,
        encoder: &mut TransferCoding,
        body_reader: &mut BodyReader,
    ) -> Result<(), Error<S::Error, BE>> {
        pin!(body);
        loop {
            match self
                .try_poll_body(body.as_mut())
                .select(self.io.ready(body_reader, &mut self.ctx))
                .await
            {
                SelectOutput::A(Some(Ok(bytes))) => encoder.encode(bytes, &mut self.io.write_buf),
                SelectOutput::B(Ok(ready)) => {
                    if ready.is_readable() {
                        if let Err(e) = self.io.try_read() {
                            body_reader.feed_error(e, &mut self.ctx);
                        }
                    }
                    if ready.is_writable() {
                        self.io.try_write()?;
                    }
                }
                SelectOutput::A(None) => {
                    // Request body is partial consumed. Close connection in case there are bytes
                    // remain in socket.
                    if !body_reader.decoder.is_eof() {
                        self.ctx.set_ctype(ConnectionType::Close);
                    }
                    encoder.encode_eof(&mut self.io.write_buf);
                    return Ok(());
                }
                SelectOutput::B(Err(e)) => return Err(e.into()),
                SelectOutput::A(Some(Err(e))) => return Err(Error::Body(e)),
            }
        }
    }

    fn try_poll_body<'b>(&self, mut body: Pin<&'b mut ResB>) -> impl Future<Output = Option<Result<Bytes, BE>>> + 'b {
        let write_backpressure = self.io.write_buf.backpressure();
        async move {
            if write_backpressure {
                pending().await
            } else {
                poll_fn(|cx| body.as_mut().poll_next(cx)).await
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn request_error<F>(&mut self, func: F) -> Result<(), Error<S::Error, BE>>
    where
        F: FnOnce() -> Response<NoneBody<Bytes>>,
    {
        self.ctx.set_ctype(ConnectionType::Close);
        let (parts, body) = func().into_parts();
        self.encode_head(parts, &body).map(|_| ())
    }
}

type DecodedHead<ReqB> = (Request<ReqB>, BodyReader);

struct BodyReader {
    decoder: TransferCoding,
    tx: RequestBodySender,
}

impl BodyReader {
    fn from_coding(decoder: TransferCoding) -> (Self, RequestBody) {
        let (tx, body) = RequestBody::channel(decoder.is_eof());
        let body_reader = BodyReader { decoder, tx };
        (body_reader, body)
    }

    // dispatcher MUST call this method before do any io reading.
    // a none ready state means the body consumer either is in backpressure or don't expect any more body.
    async fn ready<D, const READ_BUF_LIMIT: usize, const HEADER_LIMIT: usize>(
        &mut self,
        read_buf: &mut FlatBuf<READ_BUF_LIMIT>,
        ctx: &mut Context<'_, D, HEADER_LIMIT>,
    ) {
        loop {
            match self.decoder.decode(&mut *read_buf) {
                ChunkResult::Ok(bytes) => self.tx.feed_data(bytes),
                // BodyReader's only error case is when service future drop the request
                // body consumer half way. In this case notify Context to close connection afterwards.
                // Service future is trusted to produce a meaningful response after it drops
                // the request body.
                ChunkResult::InsufficientData => match self.tx.ready().await {
                    Ok(_) => return,
                    Err(_) => self.set_close(ctx),
                },
                ChunkResult::Eof => self.tx.feed_eof(),
                ChunkResult::AlreadyEof => pending().await,
                ChunkResult::Err(e) => self.feed_error(e, ctx),
            }
        }
    }

    // feed error to body sender and prepare for close connection.
    #[cold]
    #[inline(never)]
    fn feed_error<D, const HEADER_LIMIT: usize>(&mut self, e: io::Error, ctx: &mut Context<'_, D, HEADER_LIMIT>) {
        self.tx.feed_error(e);
        self.set_close(ctx);
    }

    // prepare for close connection by end decoder and set context to closed connection regardless their current states.
    #[cold]
    #[inline(never)]
    fn set_close<D, const HEADER_LIMIT: usize>(&mut self, ctx: &mut Context<'_, D, HEADER_LIMIT>) {
        self.decoder.set_eof();
        ctx.set_ctype(ConnectionType::Close);
    }

    async fn wait_for_poll(&mut self) {
        // The error case is the same condition as Self::ready method.
        // Remote should not have sent any body until 100 continue is received
        // which means it's safe(possibly) to keep the connection open on error path.
        if self.tx.wait_for_poll().await.is_err() {
            // like Self::ready method. just pending on error path
            pending().await
        }
    }
}
