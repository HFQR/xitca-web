use std::{io, rc::Rc};

use core::{
    convert::Infallible,
    future::{pending, poll_fn},
    marker::PhantomData,
    net::SocketAddr,
    pin::{Pin, pin},
    time::Duration,
};

use futures_core::stream::Stream;
use tokio_util::sync::CancellationToken;
use tracing::trace;
use xitca_io::io::{AsyncIo, Interest, Ready};
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{
    body::NoneBody,
    bytes::{Bytes, EitherBuf},
    config::HttpServiceConfig,
    date::DateTime,
    h1::{
        body::{RequestBody, RequestBodySender},
        error::Error,
    },
    http::{
        StatusCode,
        response::{Parts, Response},
    },
    util::{
        buffered::{BufferedIo, ListWriteBuf, ReadBuf, WriteBuf},
        timer::{KeepAlive, Timeout},
    },
};

use super::proto::{
    buf_write::H1BufWrite,
    codec::{ChunkResult, TransferCoding},
    context::Context,
    encode::CONTINUE,
    error::ProtoError,
};
use crate::util::futures::WaitOrPending;

type ExtRequest<B> = crate::http::Request<crate::http::RequestExt<B>>;

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
    cancellation_token: CancellationToken,
) -> Result<(), Error<S::Error, BE>>
where
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    St: AsyncIo,
    D: DateTime,
{
    let write_buf = if config.vectored_write && io.is_vectored_write() {
        EitherBuf::Left(ListWriteBuf::<_, WRITE_BUF_LIMIT>::default())
    } else {
        EitherBuf::Right(WriteBuf::<WRITE_BUF_LIMIT>::default())
    };

    Dispatcher::new(io, addr, timer, config, service, date, write_buf, cancellation_token)
        .run()
        .await
}

/// Http/1 dispatcher
struct Dispatcher<'a, St, S, ReqB, W, D, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize> {
    io: BufferedIo<'a, St, W, READ_BUF_LIMIT>,
    timer: Timer<'a>,
    ctx: Context<'a, D, HEADER_LIMIT>,
    service: &'a S,
    _phantom: PhantomData<ReqB>,
    cancellation_token: CancellationToken,
    request_guard: Rc<()>,
}

// timer state is transformed in following order:
//
// Idle (expecting keep-alive duration)           <--
//  |                                               |
//  --> Wait (expecting request head duration)      |
//       |                                          |
//       --> Throttle (expecting manually set to Idle again)
enum TimerState {
    Idle,
    Wait,
    Throttle,
}

pub(super) struct Timer<'a> {
    timer: Pin<&'a mut KeepAlive>,
    state: TimerState,
    ka_dur: Duration,
    req_dur: Duration,
}

impl<'a> Timer<'a> {
    pub(super) fn new(timer: Pin<&'a mut KeepAlive>, ka_dur: Duration, req_dur: Duration) -> Self {
        Self {
            timer,
            state: TimerState::Idle,
            ka_dur,
            req_dur,
        }
    }

    pub(super) fn reset_state(&mut self) {
        self.state = TimerState::Idle;
    }

    pub(super) fn get(&mut self) -> Pin<&mut KeepAlive> {
        self.timer.as_mut()
    }

    // update timer with a given base instant value. the final deadline is calculated base on it.
    pub(super) fn update(&mut self, now: tokio::time::Instant) {
        let dur = match self.state {
            TimerState::Idle => {
                self.state = TimerState::Wait;
                self.ka_dur
            }
            TimerState::Wait => {
                self.state = TimerState::Throttle;
                self.req_dur
            }
            TimerState::Throttle => return,
        };
        self.timer.as_mut().update(now + dur)
    }

    #[cold]
    #[inline(never)]
    pub(super) fn map_to_err<SE, BE>(&self) -> Error<SE, BE> {
        match self.state {
            TimerState::Wait => Error::KeepAliveExpire,
            TimerState::Throttle => Error::RequestTimeout,
            TimerState::Idle => unreachable!(),
        }
    }
}

impl<'a, St, S, ReqB, ResB, BE, W, D, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize>
    Dispatcher<'a, St, S, ReqB, W, D, HEADER_LIMIT, READ_BUF_LIMIT>
where
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    St: AsyncIo,
    W: H1BufWrite,
    D: DateTime,
{
    #[allow(clippy::too_many_arguments)]
    fn new<const WRITE_BUF_LIMIT: usize>(
        io: &'a mut St,
        addr: SocketAddr,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
        service: &'a S,
        date: &'a D,
        write_buf: W,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            io: BufferedIo::new(io, write_buf),
            timer: Timer::new(timer, config.keep_alive_timeout, config.request_head_timeout),
            ctx: Context::with_addr(addr, date),
            service,
            _phantom: PhantomData,
            cancellation_token,
            request_guard: Rc::new(()),
        }
    }

    async fn run(mut self) -> Result<(), Error<S::Error, BE>> {
        loop {
            match self._run().await {
                Ok(_) => {}
                Err(Error::KeepAliveExpire) => {
                    trace!(target: "h1_dispatcher", "Connection keep-alive expired. Shutting down");
                    return Ok(());
                }
                Err(Error::RequestTimeout) => self.request_error(|| status_only(StatusCode::REQUEST_TIMEOUT)),
                Err(Error::Proto(ProtoError::HeaderTooLarge)) => {
                    self.request_error(|| status_only(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE))
                }
                Err(Error::Proto(_)) => self.request_error(|| status_only(StatusCode::BAD_REQUEST)),
                Err(e) => return Err(e),
            };

            // TODO: add timeout for drain write?
            self.io.drain_write().await?;

            // shutdown io if connection is closed.
            if self.ctx.is_connection_closed() {
                return self.io.shutdown().await.map_err(Into::into);
            }

            // shutdown io if there is no more read buf
            if self.io.read_buf.is_empty()
                && self.cancellation_token.is_cancelled()
                && Rc::strong_count(&self.request_guard) == 1
            {
                return self.io.shutdown().await.map_err(Into::into);
            }
        }
    }

    async fn _run(&mut self) -> Result<(), Error<S::Error, BE>> {
        self.timer.update(self.ctx.date().now());

        match self
            .io
            .read()
            .select(WaitOrPending::new(
                self.cancellation_token.cancelled(),
                self.cancellation_token.is_cancelled(),
            ))
            .timeout(self.timer.get())
            .await
        {
            Err(_) => return Err(self.timer.map_to_err()),
            Ok(SelectOutput::A(Ok(_))) => {}
            Ok(SelectOutput::A(Err(_))) => return Err(Error::KeepAliveExpire),
            Ok(SelectOutput::B(())) => {}
        }

        while let Some((req, decoder)) = self.ctx.decode_head::<READ_BUF_LIMIT>(&mut self.io.read_buf)? {
            self.timer.reset_state();

            let (mut body_reader, body) = BodyReader::from_coding(decoder);
            let req = req.map(|ext| ext.map_body(|_| ReqB::from(body)));
            let _guard = self.request_guard.clone();

            let (parts, body) = match self
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

            let encoder = &mut self.encode_head(parts, &body)?;
            let mut body = pin!(body);

            loop {
                match self
                    .try_poll_body(body.as_mut())
                    .select(self.io_ready(&mut body_reader))
                    .await
                {
                    SelectOutput::A(Some(Ok(bytes))) => encoder.encode(bytes, &mut self.io.write_buf),
                    SelectOutput::B(Ok(ready)) => {
                        if ready.is_readable() {
                            if let Err(e) = self.io.try_read() {
                                body_reader.feed_error(e);
                            }
                        }
                        if ready.is_writable() {
                            self.io.try_write()?;
                        }
                    }
                    SelectOutput::A(None) => {
                        encoder.encode_eof(&mut self.io.write_buf);
                        break;
                    }
                    SelectOutput::B(Err(e)) => return Err(e.into()),
                    SelectOutput::A(Some(Err(e))) => return Err(Error::Body(e)),
                }
            }

            if !body_reader.decoder.is_eof() {
                self.ctx.set_close();
                break;
            }
        }

        Ok(())
    }

    fn encode_head(&mut self, parts: Parts, body: &impl Stream) -> Result<TransferCoding, ProtoError> {
        self.ctx.encode_head(parts, body, &mut self.io.write_buf)
    }

    // an associated future of self.service that runs until service is resolved or error produced.
    async fn request_body_handler(&mut self, body_reader: &mut BodyReader) -> Result<Infallible, Error<S::Error, BE>> {
        if self.ctx.is_expect_header() {
            // wait for service future to start polling RequestBody.
            if body_reader.wait_for_poll().await.is_ok() {
                // encode continue as service future want a body.
                self.io.write_buf.write_buf_static(CONTINUE);
                // use drain write to make sure continue is sent to client.
                self.io.drain_write().await?;
            }
        }

        loop {
            body_reader.ready(&mut self.io.read_buf).await;
            self.io.read().await?;
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

    // Check readable and writable state of BufferedIo and ready state of request body reader.
    // return error when runtime is shutdown.(See AsyncIo::ready for reason).
    async fn io_ready(&mut self, body_reader: &mut BodyReader) -> io::Result<Ready> {
        if !self.io.write_buf.want_write_io() {
            body_reader.ready(&mut self.io.read_buf).await;
            self.io.io.ready(Interest::READABLE).await
        } else {
            match body_reader
                .ready(&mut self.io.read_buf)
                .select(self.io.io.ready(Interest::WRITABLE))
                .await
            {
                SelectOutput::A(_) => self.io.io.ready(Interest::READABLE | Interest::WRITABLE).await,
                SelectOutput::B(res) => res,
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn request_error(&mut self, func: impl FnOnce() -> Response<NoneBody<Bytes>>) {
        self.ctx.set_close();
        let (parts, body) = func().into_parts();
        self.encode_head(parts, &body).expect("request_error must be correct");
    }
}

pub(super) struct BodyReader {
    pub(super) decoder: TransferCoding,
    tx: RequestBodySender,
}

impl BodyReader {
    pub(super) fn from_coding(decoder: TransferCoding) -> (Self, RequestBody) {
        let (tx, body) = RequestBody::channel(decoder.is_eof());
        let body_reader = BodyReader { decoder, tx };
        (body_reader, body)
    }

    // dispatcher MUST call this method before do any io reading.
    // a none ready state means the body consumer either is in backpressure or don't expect body.
    pub(super) async fn ready<const READ_BUF_LIMIT: usize>(&mut self, read_buf: &mut ReadBuf<READ_BUF_LIMIT>) {
        loop {
            match self.decoder.decode(&mut *read_buf) {
                ChunkResult::Ok(bytes) => self.tx.feed_data(bytes),
                ChunkResult::InsufficientData => match self.tx.ready().await {
                    Ok(_) => return,
                    // service future drop RequestBody so marker decoder to corrupted.
                    Err(_) => self.decoder.set_corrupted(),
                },
                ChunkResult::OnEof => self.tx.feed_eof(),
                ChunkResult::AlreadyEof | ChunkResult::Corrupted => pending().await,
                ChunkResult::Err(e) => self.feed_error(e),
            }
        }
    }

    // feed error to body sender and prepare for close connection.
    #[cold]
    #[inline(never)]
    pub(super) fn feed_error(&mut self, e: io::Error) {
        self.tx.feed_error(e);
        self.decoder.set_corrupted();
    }

    // wait for service start to consume RequestBody.
    pub(super) async fn wait_for_poll(&mut self) -> io::Result<()> {
        // IMPORTANT: service future drop RequestBody so marker decoder to corrupted.
        self.tx
            .wait_for_poll()
            .await
            .inspect_err(|_| self.decoder.set_corrupted())
    }
}

#[cold]
#[inline(never)]
pub(super) fn status_only(status: StatusCode) -> Response<NoneBody<Bytes>> {
    Response::builder().status(status).body(NoneBody::default()).unwrap()
}
