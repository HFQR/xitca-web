use core::{
    cell::RefCell,
    future::poll_fn,
    marker::PhantomData,
    mem,
    net::SocketAddr,
    pin::{Pin, pin},
    task::{self, Poll, Waker, ready},
    time::Duration,
};

use std::{io, rc::Rc};

use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use tracing::trace;
use xitca_io::io::{AsyncIo, Interest};
use xitca_service::Service;
use xitca_unsafe_collection::futures::SelectOutput;

use crate::{
    body::NoneBody,
    bytes::{Bytes, BytesMut, EitherBuf},
    config::HttpServiceConfig,
    date::DateTime,
    h1::{body::RequestBody, error::Error, proto::encode::CONTINUE_BYTES},
    http::{StatusCode, response::Response},
    util::{
        buffered::{ListWriteBuf, WriteBuf},
        timer::{KeepAlive, Timeout},
    },
};

use super::proto::{
    buf_write::H1BufWrite,
    codec::{ChunkResult, TransferCoding},
    context::Context,
    error::ProtoError,
};

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
    io: St,
    addr: SocketAddr,
    timer: Pin<&'a mut KeepAlive>,
    config: HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
    service: &'a S,
    date: &'a D,
) -> Result<(), Error<S::Error, BE>>
where
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    St: AsyncIo + 'static,
    for<'i> &'i St: AsyncIo,
    D: DateTime,
{
    let write_buf = if config.vectored_write && (&io).is_vectored_write() {
        EitherBuf::Left(ListWriteBuf::<_, WRITE_BUF_LIMIT>::default())
    } else {
        EitherBuf::Right(WriteBuf::<WRITE_BUF_LIMIT>::default())
    };

    Dispatcher::new(io, addr, timer, config, service, date, write_buf)
        .run()
        .await
}

/// Http/1 dispatcher
struct Dispatcher<'a, Io, S, ReqB, W, D, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize> {
    io: Rc<Io>,
    read_buf: BytesMut,
    write_buf: W,
    timer: Timer<'a>,
    ctx: Context<'a, D, HEADER_LIMIT>,
    notify: Notify<BytesMut>,
    service: &'a S,
    _phantom: PhantomData<ReqB>,
}

impl<'a, St, S, ReqB, ResB, BE, W, D, const H_LIMIT: usize, const R_LIMIT: usize>
    Dispatcher<'a, St, S, ReqB, W, D, H_LIMIT, R_LIMIT>
where
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    St: AsyncIo + 'static,
    for<'i> &'i St: AsyncIo,
    W: H1BufWrite,
    D: DateTime,
{
    fn new<const W_LIMIT: usize>(
        io: St,
        addr: SocketAddr,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<H_LIMIT, R_LIMIT, W_LIMIT>,
        service: &'a S,
        date: &'a D,
        write_buf: W,
    ) -> Self {
        Self {
            io: Rc::new(io),
            read_buf: BytesMut::new(),
            write_buf,
            timer: Timer::new(timer, config.keep_alive_timeout, config.request_head_timeout),
            ctx: Context::with_addr(addr, date),
            notify: Notify::new(),
            service,
            _phantom: PhantomData,
        }
    }

    async fn run(mut self) -> Result<(), Error<S::Error, BE>> {
        loop {
            if let Err(err) = self._run().await {
                handle_error(&mut self.ctx, &mut self.write_buf, err)?;
            }

            // TODO: add timeout for drain write?
            write(&*self.io, &mut self.write_buf).await?;

            if self.ctx.is_connection_closed() {
                return self.shutdown().await.map_err(Into::into);
            }
        }
    }

    async fn _run(&mut self) -> Result<(), Error<S::Error, BE>> {
        self.timer.update(self.ctx.date().now());

        let read = read(&*self.io, &mut self.read_buf)
            .timeout(self.timer.get())
            .await
            .map_err(|_| self.timer.map_to_err())??;

        if read == 0 {
            self.ctx.set_close();
            return Ok(());
        }

        while let Some((req, decoder)) = self.ctx.decode_head::<R_LIMIT>(&mut self.read_buf)? {
            self.timer.reset_state();

            let (waiter, body) = if decoder.is_eof() {
                (None, RequestBody::default())
            } else {
                let body = body(
                    self.io.clone(),
                    self.ctx.is_expect_header(),
                    R_LIMIT,
                    decoder,
                    mem::take(&mut self.read_buf),
                    self.notify.notifier(),
                );

                (Some(&mut self.notify), body)
            };

            let req = req.map(|ext| ext.map_body(|_| ReqB::from(body)));

            let (parts, body) = self.service.call(req).await.map_err(Error::Service)?.into_parts();

            let mut encoder = self.ctx.encode_head(parts, &body, &mut self.write_buf)?;

            // this block is necessary. ResB has to be dropped asap as it may hold ownership of
            // Body type which if not dropped before Notifier::notify is called would prevent
            // Notifier from waking up Notify.
            {
                let mut body = pin!(body);

                loop {
                    let buf = &mut self.write_buf;

                    let res = poll_fn(|cx| match body.as_mut().poll_next(cx) {
                        Poll::Ready(res) => Poll::Ready(SelectOutput::A(res)),
                        Poll::Pending if buf.want_write_io() => Poll::Pending,
                        Poll::Pending => Poll::Ready(SelectOutput::B(())),
                    })
                    .await;

                    match res {
                        SelectOutput::A(Some(Ok(bytes))) => {
                            encoder.encode(bytes, buf);
                            if buf.want_write_buf() {
                                continue;
                            }
                        }
                        SelectOutput::A(Some(Err(e))) => return self.on_body_error(e).await,
                        SelectOutput::A(None) => break encoder.encode_eof(buf),
                        SelectOutput::B(_) => {}
                    }

                    write(&*self.io, buf).await?;
                }
            }

            if let Some(waiter) = waiter {
                match waiter.wait().await {
                    Some(read_buf) => self.read_buf = read_buf,
                    None => {
                        self.ctx.set_close();
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    #[cold]
    #[inline(never)]
    async fn shutdown(self) -> Result<(), Error<S::Error, BE>> {
        let mut io = Rc::try_unwrap(self.io)
            .ok()
            .expect("Dispatcher must have exclusive ownership to Io when closing connection");

        poll_fn(|cx| Pin::new(&mut io).poll_shutdown(cx))
            .await
            .map_err(Into::into)
    }

    #[cold]
    #[inline(never)]
    async fn on_body_error(&mut self, e: BE) -> Result<(), Error<S::Error, BE>> {
        write(&*self.io, &mut self.write_buf).await?;
        Err(Error::Body(e))
    }
}

async fn read(mut io: impl AsyncIo, buf: &mut BytesMut) -> io::Result<usize> {
    buf.reserve(4096);
    loop {
        io.ready(Interest::READABLE).await?;

        match xitca_unsafe_collection::bytes::read_buf(&mut io, buf) {
            Ok(n) => return Ok(n),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
}

async fn write<W>(mut io: impl AsyncIo, buf: &mut W) -> io::Result<()>
where
    W: H1BufWrite,
{
    while buf.want_write_io() {
        io.ready(Interest::WRITABLE).await?;
        buf.do_io(&mut io)?;
    }

    Ok(())
}

#[cold]
#[inline(never)]
pub(super) fn handle_error<D, W, S, B, const H_LIMIT: usize>(
    ctx: &mut Context<'_, D, H_LIMIT>,
    buf: &mut W,
    err: Error<S, B>,
) -> Result<(), Error<S, B>>
where
    D: DateTime,
    W: H1BufWrite,
{
    ctx.set_close();
    match err {
        Error::KeepAliveExpire => {
            trace!(target: "h1_dispatcher", "Connection keep-alive expired. Shutting down")
        }
        e => {
            let status = match e {
                Error::RequestTimeout => StatusCode::REQUEST_TIMEOUT,
                Error::Proto(ProtoError::HeaderTooLarge) => StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                Error::Proto(_) => StatusCode::BAD_REQUEST,
                e => return Err(e),
            };
            let (parts, body) = Response::builder()
                .status(status)
                .body(NoneBody::<Bytes>::default())
                .unwrap()
                .into_parts();
            ctx.encode_head(parts, &body, buf)
                .expect("request_error must be correct");
        }
    }
    Ok(())
}

fn body<Io>(
    io: Rc<Io>,
    is_expect: bool,
    limit: usize,
    decoder: TransferCoding,
    read_buf: BytesMut,
    notify: Notifier<BytesMut>,
) -> RequestBody
where
    Io: 'static,
    for<'i> &'i Io: AsyncIo,
{
    let body = BodyInner {
        io,
        decoder: Decoder {
            decoder,
            limit,
            read_buf,
            notify,
        },
    };

    let state = if is_expect {
        State::ExpectWrite {
            fut: async {
                write(&*body.io, &mut BytesMut::from(CONTINUE_BYTES))
                    .await
                    .map(|_| body)
            },
        }
    } else {
        State::Body { body }
    };

    RequestBody::new(BodyReader { state })
}

pin_project! {
    #[project = StateProj]
    #[project_replace = StateProjReplace]
    enum State<Io, FutE> {
        Body {
            body: BodyInner<Io>
        },
        ExpectWrite {
            #[pin]
            fut: FutE,
        },
        None,
    }
}

pin_project! {
    struct BodyReader<Io,  FutE> {
        #[pin]
        state: State<Io, FutE>
    }
}

struct BodyInner<Io> {
    io: Rc<Io>,
    decoder: Decoder,
}

impl<Io, FutE> Stream for BodyReader<Io, FutE>
where
    for<'i> &'i Io: AsyncIo,
    FutE: Future<Output = io::Result<BodyInner<Io>>>,
{
    type Item = io::Result<Bytes>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            match this.state.as_mut().project() {
                StateProj::Body { body } => {
                    let mut io = &*body.io;
                    let decoder = &mut body.decoder;

                    match decoder.decode() {
                        ChunkResult::Ok(bytes) => return Poll::Ready(Some(Ok(bytes))),
                        ChunkResult::Err(e) => return Poll::Ready(Some(Err(e))),
                        ChunkResult::InsufficientData => decoder.limit_check()?,
                        _ => return Poll::Ready(None),
                    }

                    ready!(io.poll_ready(Interest::READABLE, cx))?;

                    match xitca_unsafe_collection::bytes::read_buf(&mut io, &mut decoder.read_buf) {
                        Ok(n) => {
                            if n == 0 {
                                this.state.as_mut().project_replace(State::None);
                                return Poll::Ready(None);
                            }
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) => return Poll::Ready(Some(Err(e))),
                    }
                }
                StateProj::ExpectWrite { fut } => {
                    let body = ready!(fut.poll(cx))?;
                    this.state.as_mut().project_replace(State::Body { body });
                }
                StateProj::None => return Poll::Ready(None),
            }
        }
    }
}

pub(super) struct Timer<'a> {
    timer: Pin<&'a mut KeepAlive>,
    state: TimerState,
    ka_dur: Duration,
    req_dur: Duration,
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

pub(super) struct Decoder {
    pub(super) decoder: TransferCoding,
    pub(super) limit: usize,
    pub(super) read_buf: BytesMut,
    pub(super) notify: Notifier<BytesMut>,
}

impl Decoder {
    pub(super) fn decode(&mut self) -> ChunkResult {
        self.decoder.decode(&mut self.read_buf)
    }

    pub(super) fn limit_check(&self) -> io::Result<()> {
        if self.read_buf.len() >= self.limit {
            let msg = format!(
                "READ_BUF_LIMIT reached: {{ limit: {}, length: {} }}",
                self.limit,
                self.read_buf.len()
            );
            Err(io::Error::other(msg))
        } else {
            Ok(())
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if self.decoder.is_eof() {
            let buf = mem::take(&mut self.read_buf);
            self.notify.notify(buf);
        }
    }
}

pub(super) struct Notify<T>(Rc<RefCell<Inner<T>>>);

impl<T> Notify<T> {
    pub(super) fn new() -> Self {
        Self(Rc::new(RefCell::new(Inner { waker: None, val: None })))
    }

    pub(super) fn notifier(&mut self) -> Notifier<T> {
        Notifier(self.0.clone())
    }

    pub(super) fn wait(&mut self) -> impl Future<Output = Option<T>> + '_ {
        poll_fn(|cx| {
            let mut inner = self.0.borrow_mut();
            if let Some(val) = inner.val.take() {
                return Poll::Ready(Some(val));
            } else if Rc::strong_count(&self.0) == 1 {
                return Poll::Ready(None);
            }
            inner.waker = Some(cx.waker().clone());
            Poll::Pending
        })
    }
}

pub(super) struct Notifier<T>(Rc<RefCell<Inner<T>>>);

impl<T> Drop for Notifier<T> {
    fn drop(&mut self) {
        if let Some(waker) = self.0.borrow_mut().waker.take() {
            waker.wake();
        }
    }
}

impl<T> Notifier<T> {
    pub(super) fn notify(&mut self, val: T) {
        self.0.borrow_mut().val = Some(val);
    }
}

struct Inner<V> {
    waker: Option<Waker>,
    val: Option<V>,
}
