use core::{
    convert::Infallible,
    future::{pending, poll_fn, Future},
    marker::PhantomData,
    ops::DerefMut,
    pin::{pin, Pin},
    time::Duration,
};

use std::{io, net::SocketAddr};

use futures_core::stream::Stream;
use tracing::trace;
use xitca_io::io::{AsyncIo, Interest, Ready};
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{
    body::NoneBody,
    bytes::Bytes,
    config::HttpServiceConfig,
    date::DateTime,
    h1::{
        body::{RequestBody, RequestBodySender},
        error::Error,
    },
    http::{
        response::{Parts, Response},
        Request, RequestExt, StatusCode,
    },
    util::{
        buffered::{BufferedIo, ListWriteBuf, ReadBuf, WriteBuf},
        hint::unlikely,
        timer::{KeepAlive, Timeout},
    },
};

use super::{
    buf_write::H1BufWrite,
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
    addr: SocketAddr,
    timer: Pin<&'a mut KeepAlive>,
    config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    service: &'a S,
    date: &'a D,
) -> Result<(), Error<S::Error, BE>>
where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    St: AsyncIo,
    D: DateTime,
{
    let is_vectored = config.vectored_write && io.is_vectored_write();

    if is_vectored {
        let write_buf = ListWriteBuf::<_, WRITE_BUF_LIMIT>::default();
        Dispatcher::new(io, addr, timer, config, service, date, write_buf)
            .run()
            .await
    } else {
        let write_buf = WriteBuf::<WRITE_BUF_LIMIT>::default();
        Dispatcher::new(io, addr, timer, config, service, date, write_buf)
            .run()
            .await
    }
}

/// Http/1 dispatcher
struct Dispatcher<'a, St, S, ReqB, W, D, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize> {
    io: BufferedIo<'a, St, W, READ_BUF_LIMIT>,
    timer: Pin<&'a mut KeepAlive>,
    ka_dur: Duration,
    ctx: Context<'a, D, HEADER_LIMIT>,
    service: &'a S,
    _phantom: PhantomData<ReqB>,
}

impl<'a, St, S, ReqB, ResB, BE, W, D, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize>
    Dispatcher<'a, St, S, ReqB, W, D, HEADER_LIMIT, READ_BUF_LIMIT>
where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    St: AsyncIo,
    W: H1BufWrite,
    D: DateTime,
{
    fn new<const WRITE_BUF_LIMIT: usize>(
        io: &'a mut St,
        addr: SocketAddr,
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
            ctx: Context::with_addr(addr, date),
            service,
            _phantom: PhantomData,
        }
    }

    async fn run(mut self) -> Result<(), Error<S::Error, BE>> {
        loop {
            match self.ctx.ctype() {
                ConnectionType::KeepAlive => self.update_timer(),
                ConnectionType::Init => unlikely(),
                ConnectionType::Close => return self.io.shutdown().await.map_err(Into::into),
            }

            match self._run().await {
                Ok(_) => {}
                Err(Error::KeepAliveExpire) => match self.ctx.ctype() {
                    ConnectionType::Init => self.request_error(timeout),
                    _ => {
                        trace!(target: "h1_dispatcher", "Connection keep-alive expired. Shutting down");
                        return Ok(());
                    }
                },
                Err(Error::Proto(ProtoError::Parse(Parse::HeaderTooLarge))) => self.request_error(header_too_large),
                Err(Error::Proto(ProtoError::Parse(_))) => self.request_error(bad_request),
                Err(e) => return Err(e),
            }

            // TODO: add timeout for drain write?
            self.io.drain_write().await?;
        }
    }

    async fn _run(&mut self) -> Result<(), Error<S::Error, BE>> {
        self.io.read().timeout(self.timer.as_mut()).await??;

        while let Some((req, decoder)) = self.ctx.decode_head::<READ_BUF_LIMIT>(self.io.read_buf.deref_mut())? {
            let (mut body_reader, body) = BodyReader::from_coding(decoder);
            let req = req.map(|ext| ext.map_body(|_| ReqB::from(body)));

            let (parts, res_body) = match self
                .service
                .call(req)
                .select(self.request_body_handler(&mut body_reader))
                .await
            {
                SelectOutput::A(Ok(res)) => res.into_parts(),
                SelectOutput::A(Err(e)) => return Err(Error::Service(e)),
                SelectOutput::B(Err(e)) => return Err(e),
                SelectOutput::B(Ok(i)) => match i {},
            };

            let encoder = &mut self.encode_head(parts, &res_body)?;
            self.response_handler(res_body, encoder, &mut body_reader).await?;
        }

        Ok(())
    }

    // update timer deadline according to keep alive duration.
    fn update_timer(&mut self) {
        let now = self.ctx.date.now() + self.ka_dur;
        self.timer.as_mut().update(now);
    }

    fn encode_head<B>(&mut self, parts: Parts, body: &B) -> Result<TransferCoding, Error<S::Error, BE>>
    where
        B: Stream,
    {
        self.ctx
            .encode_head(parts, body, &mut self.io.write_buf)
            .map_err(Into::into)
    }

    // an associated future of self.service that runs until service is resolved or error produced.
    async fn request_body_handler(&mut self, body_reader: &mut BodyReader) -> Result<Infallible, Error<S::Error, BE>> {
        if self.ctx.is_expect_header() {
            // wait for service future to start polling RequestBody.
            if body_reader.wait_for_poll().await.is_ok() {
                // encode continue as service future want a body.
                self.ctx.encode_continue(&mut self.io.write_buf);
                // use drain write to make sure continue is sent to client.
                self.io.drain_write().await?;
            }
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
        let mut body = pin!(body);
        loop {
            match self
                .try_poll_body(body.as_mut())
                .select(io_ready(&mut self.io, body_reader, &mut self.ctx))
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
                    if !body_reader.decoder.is_eof() {
                        // request body is partial consumed. close connection in case there are
                        // bytes remain in socket.
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
        let want_buf = self.io.write_buf.want_write_buf();
        async move {
            if want_buf {
                poll_fn(|cx| body.as_mut().poll_next(cx)).await
            } else {
                pending().await
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn request_error<F>(&mut self, func: F)
    where
        F: FnOnce() -> Response<NoneBody<Bytes>>,
    {
        self.ctx.set_ctype(ConnectionType::Close);
        let (parts, body) = func().into_parts();
        assert!(self.encode_head(parts, &body).is_ok(), "request_error must be correct");
    }
}

// Check readable and writable state of BufferedIo and ready state of request body reader.
// return error when runtime is shutdown.(See AsyncIo::ready for reason).
async fn io_ready<St, W, D, const READ_BUF_LIMIT: usize, const HEADER_LIMIT: usize>(
    io: &mut BufferedIo<'_, St, W, READ_BUF_LIMIT>,
    body_reader: &mut BodyReader,
    ctx: &mut Context<'_, D, HEADER_LIMIT>,
) -> io::Result<Ready>
where
    St: AsyncIo,
    W: H1BufWrite,
{
    if !io.write_buf.want_write_io() {
        body_reader.ready(&mut io.read_buf, ctx).await;
        io.io.ready(Interest::READABLE).await
    } else {
        match body_reader
            .ready(&mut io.read_buf, ctx)
            .select(io.io.ready(Interest::WRITABLE))
            .await
        {
            SelectOutput::A(_) => io.io.ready(Interest::READABLE | Interest::WRITABLE).await,
            SelectOutput::B(res) => res,
        }
    }
}

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
    // a none ready state means the body consumer either is in backpressure or don't expect body.
    async fn ready<D, const READ_BUF_LIMIT: usize, const HEADER_LIMIT: usize>(
        &mut self,
        read_buf: &mut ReadBuf<READ_BUF_LIMIT>,
        ctx: &mut Context<'_, D, HEADER_LIMIT>,
    ) {
        loop {
            match self.decoder.decode(&mut *read_buf) {
                ChunkResult::Ok(bytes) => self.tx.feed_data(bytes),
                ChunkResult::InsufficientData => match self.tx.ready().await {
                    Ok(_) => return,
                    // service future drop RequestBody half way so notify Context to close
                    // connection afterwards.
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

    // wait for service start to consume RequestBody.
    async fn wait_for_poll(&mut self) -> io::Result<()> {
        self.tx.wait_for_poll().await.map_err(|e| {
            // IMPORTANT: service future drop RequestBody so set decoder to eof.
            self.decoder.set_eof();
            e
        })
    }
}

fn header_too_large() -> Response<NoneBody<Bytes>> {
    status_only(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE)
}

fn timeout() -> Response<NoneBody<Bytes>> {
    status_only(StatusCode::REQUEST_TIMEOUT)
}

fn bad_request() -> Response<NoneBody<Bytes>> {
    status_only(StatusCode::BAD_REQUEST)
}

#[cold]
#[inline(never)]
fn status_only(status: StatusCode) -> Response<NoneBody<Bytes>> {
    Response::builder().status(status).body(NoneBody::default()).unwrap()
}
