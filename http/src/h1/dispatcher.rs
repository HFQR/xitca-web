use core::{
    cell::RefCell,
    future::poll_fn,
    marker::PhantomData,
    mem,
    net::SocketAddr,
    ops::{Deref, DerefMut},
    pin::{Pin, pin},
    task::{self, Poll, Waker, ready},
    time::Duration,
};

use std::{io, rc::Rc};

use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use tracing::trace;
use xitca_io::bytes::BytesMut;
use xitca_io::{
    bytes::Buf,
    io::{AsyncIo, Interest},
};
use xitca_service::Service;
use xitca_unsafe_collection::futures::SelectOutput;

use crate::{
    body::NoneBody,
    bytes::Bytes,
    config::HttpServiceConfig,
    date::DateTime,
    h1::{body::RequestBody, error::Error},
    http::{StatusCode, response::Response},
    util::timer::{KeepAlive, Timeout},
};

use super::{
    body::Body,
    proto::{
        codec::{ChunkResult, TransferCoding},
        context::Context,
        encode::CONTINUE_BYTES,
        error::ProtoError,
    },
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
    const H_LIMIT: usize,
    const R_LIMIT: usize,
    const W_LIMIT: usize,
>(
    io: St,
    addr: SocketAddr,
    timer: Pin<&'a mut KeepAlive>,
    config: HttpServiceConfig<H_LIMIT, R_LIMIT, W_LIMIT>,
    service: &'a S,
    date: &'a D,
) -> Result<(), Error<S::Error, BE>>
where
    St: AsyncIo + 'static,
    for<'i> &'i St: AsyncIo,
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    D: DateTime,
{
    // TODO: re-enable vectored write
    // let write_buf = if config.vectored_write && (&io).is_vectored_write() {
    //     EitherBuf::Left(ListWriteBuf::<_, WRITE_BUF_LIMIT>::default())
    // } else {
    //     EitherBuf::Right(WriteBuf::<WRITE_BUF_LIMIT>::default())
    // };

    let mut dispatcher = Dispatcher::<_, _, _, _, H_LIMIT, R_LIMIT, W_LIMIT> {
        io: Rc::new(io),
        timer: Timer::new(timer, config.keep_alive_timeout, config.request_head_timeout),
        ctx: Context::with_addr(addr, date),
        service,
        read_buf: BufOwned::new(),
        write_buf: BytesMut::new(),
        notify: Notify::new(),
        _phantom: PhantomData,
    };

    loop {
        match dispatcher._run().await {
            Ok(_) => {}
            Err(Error::KeepAliveExpire) => {
                trace!(target: "h1_dispatcher", "Connection keep-alive expired. Shutting down");
                dispatcher.ctx.set_close();
            }
            Err(Error::RequestTimeout) => dispatcher.request_error(|| status_only(StatusCode::REQUEST_TIMEOUT)),
            Err(Error::Proto(ProtoError::HeaderTooLarge)) => {
                dispatcher.request_error(|| status_only(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE))
            }
            Err(Error::Proto(_)) => dispatcher.request_error(|| status_only(StatusCode::BAD_REQUEST)),
            Err(e) => return Err(e),
        }

        write(&*dispatcher.io, &mut dispatcher.write_buf).await?;

        if dispatcher.ctx.is_connection_closed() {
            let io = Rc::try_unwrap(dispatcher.io)
                .ok()
                .expect("Dispatcher must have exclusive ownership to Io when closing connection");
            let mut io = pin!(io);
            return poll_fn(|cx| io.as_mut().poll_shutdown(cx)).await.map_err(Into::into);
        }
    }
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

#[cold]
#[inline(never)]
pub(super) fn status_only(status: StatusCode) -> Response<NoneBody<Bytes>> {
    Response::builder().status(status).body(NoneBody::default()).unwrap()
}

pub(super) struct Dispatcher<'a, Io, S, ReqB, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize> {
    io: Rc<Io>,
    timer: Timer<'a>,
    ctx: Context<'a, D, H_LIMIT>,
    service: &'a S,
    read_buf: BufOwned,
    write_buf: BytesMut,
    notify: Notify<BufOwned>,
    _phantom: PhantomData<ReqB>,
}

impl<'a, Io, S, ReqB, ResB, BE, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize>
    Dispatcher<'a, Io, S, ReqB, D, H_LIMIT, R_LIMIT, W_LIMIT>
where
    Io: 'static,
    for<'i> &'i Io: AsyncIo,
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    D: DateTime,
{
    async fn _run(&mut self) -> Result<(), Error<S::Error, BE>> {
        self.timer.update(self.ctx.date().now());

        let n = read(&*self.io, &mut *self.read_buf)
            .timeout(self.timer.get())
            .await
            .map_err(|_| self.timer.map_to_err())??;

        if n == 0 {
            self.ctx.set_close();
            return Ok(());
        }

        while let Some((req, decoder)) = self.ctx.decode_head::<R_LIMIT>(&mut self.read_buf)? {
            self.timer.reset_state();

            let (waiter, body) = if decoder.is_eof() {
                (None, RequestBody::default())
            } else {
                let body = Body::new::<_, R_LIMIT>(
                    self.io.clone(),
                    self.ctx.is_expect_header(),
                    decoder,
                    std::mem::take(&mut self.read_buf),
                    self.notify.notifier(),
                );

                (Some(&mut self.notify), RequestBody::new(body))
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
                        Poll::Pending if buf.is_empty() => Poll::Pending,
                        Poll::Pending => Poll::Ready(SelectOutput::B(())),
                    })
                    .await;

                    match res {
                        SelectOutput::A(Some(Ok(bytes))) => {
                            encoder.encode(bytes, buf);
                            if buf.len() < W_LIMIT {
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
    async fn on_body_error(&mut self, e: BE) -> Result<(), Error<S::Error, BE>> {
        write(&*self.io, &mut self.write_buf).await?;
        Err(Error::Body(e))
    }

    #[cold]
    #[inline(never)]
    fn request_error(&mut self, func: impl FnOnce() -> Response<NoneBody<Bytes>>) {
        self.ctx.set_close();
        let (parts, body) = func().into_parts();
        self.ctx
            .encode_head(parts, &body, &mut self.write_buf)
            .expect("request_error must be correct");
    }
}

async fn read(mut io: impl AsyncIo, buf: &mut BytesMut) -> io::Result<usize> {
    loop {
        io.ready(Interest::READABLE).await?;

        match xitca_unsafe_collection::bytes::read_buf(&mut io, buf) {
            Ok(n) => return Ok(n),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
}

async fn write(mut io: impl AsyncIo, mut buf: impl Buf) -> io::Result<()> {
    while !buf.chunk().is_empty() {
        io.ready(Interest::WRITABLE).await?;

        match std::io::Write::write(&mut io, buf.chunk()) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => buf.advance(n),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }

    while let Err(e) = std::io::Write::flush(&mut io) {
        match e.kind() {
            io::ErrorKind::WouldBlock => {
                io.ready(Interest::WRITABLE).await?;
            }
            _ => return Err(e),
        }
    }

    Ok(())
}

#[derive(Default)]
pub(super) struct BufOwned {
    pub(super) buf: Option<BytesMut>,
}

impl Deref for BufOwned {
    type Target = BytesMut;

    fn deref(&self) -> &Self::Target {
        self.buf.as_ref().unwrap()
    }
}

impl DerefMut for BufOwned {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buf.as_mut().unwrap()
    }
}

impl BufOwned {
    pub(super) fn new() -> Self {
        Self {
            buf: Some(BytesMut::new()),
        }
    }
}

impl Body {
    fn new<Io, const LIMIT: usize>(
        io: Rc<Io>,
        is_expect: bool,
        decoder: TransferCoding,
        read_buf: BufOwned,
        notify: Notifier<BufOwned>,
    ) -> Self
    where
        Io: 'static,
        for<'i> &'i Io: AsyncIo,
    {
        let body = BodyInner {
            io,
            decoder: Decoder {
                decoder,
                limit: LIMIT,
                read_buf,
                notify,
            },
        };

        let state = if is_expect {
            State::ExpectWrite {
                fut: async { write(&*body.io, CONTINUE_BYTES).await.map(|_| body) },
            }
        } else {
            State::Body { body }
        };

        Self::_new(BodyReader { state })
    }
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
    }
}

pin_project! {
    struct BodyReader<Io, FutE> {
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
                    match body.decoder.decoder.decode(&mut body.decoder.read_buf) {
                        ChunkResult::Ok(bytes) => return Poll::Ready(Some(Ok(bytes))),
                        ChunkResult::Err(e) => return Poll::Ready(Some(Err(e))),
                        ChunkResult::InsufficientData => {
                            if body.decoder.read_buf.len() > body.decoder.limit {
                                let msg = format!(
                                    "READ_BUF_LIMIT reached: {{ limit: {}, length: {} }}",
                                    body.decoder.limit,
                                    body.decoder.read_buf.len()
                                );
                                return Poll::Ready(Some(Err(io::Error::other(msg))));
                            }
                        }
                        _ => return Poll::Ready(None),
                    };

                    ready!((&*body.io).poll_ready(Interest::READABLE, cx))?;

                    match xitca_unsafe_collection::bytes::read_buf(&mut &*body.io, &mut *body.decoder.read_buf) {
                        Ok(0) => return Poll::Ready(None),
                        Ok(_) => {}
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) => return Poll::Ready(Some(Err(e))),
                    }
                }
                StateProj::ExpectWrite { fut } => {
                    let body = ready!(fut.poll(cx))?;
                    this.state.as_mut().project_replace(State::Body { body });
                }
            }
        }
    }
}

pub(super) struct Decoder {
    pub(super) decoder: TransferCoding,
    pub(super) limit: usize,
    pub(super) read_buf: BufOwned,
    pub(super) notify: Notifier<BufOwned>,
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
