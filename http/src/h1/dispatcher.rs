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

use std::{io, net::Shutdown, rc::Rc};

use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use tracing::trace;
use xitca_io::io::{AsyncBufRead, AsyncBufWrite, BoundedBuf, write_all};
use xitca_service::Service;
use xitca_unsafe_collection::futures::SelectOutput;

use crate::{
    body::NoneBody,
    bytes::{Bytes, BytesMut},
    config::HttpServiceConfig,
    date::DateTime,
    h1::{
        body::RequestBody,
        error::Error,
        proto::{buf_write::H1BufWrite, error::ProtoError},
    },
    http::{StatusCode, response::Response},
    util::timer::{KeepAlive, Timeout},
};

use super::proto::{
    codec::{ChunkResult, TransferCoding},
    context::Context,
    encode::CONTINUE_BYTES,
};

type ExtRequest<B> = crate::http::Request<crate::http::RequestExt<B>>;

/// Http/1 dispatcher
pub struct Dispatcher<'a, Io, S, ReqB, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize> {
    io: SharedIo<Io>,
    timer: Timer<'a>,
    ctx: Context<'a, D, H_LIMIT>,
    service: &'a S,
    _phantom: PhantomData<ReqB>,
}

trait BufIo {
    fn read(self, io: &impl AsyncBufRead) -> impl Future<Output = (io::Result<usize>, Self)>;

    fn write(self, io: &impl AsyncBufWrite) -> impl Future<Output = (io::Result<()>, Self)>;
}

impl BufIo for BytesMut {
    async fn read(mut self, io: &impl AsyncBufRead) -> (io::Result<usize>, Self) {
        let len = self.len();

        self.reserve(4096);

        let (res, buf) = io.read(self.slice(len..)).await;
        (res, buf.into_inner())
    }

    async fn write(self, io: &impl AsyncBufWrite) -> (io::Result<()>, Self) {
        let (res, mut buf) = write_all(io, self).await;
        buf.clear();
        (res, buf)
    }
}

impl<'a, Io, S, ReqB, ResB, BE, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize>
    Dispatcher<'a, Io, S, ReqB, D, H_LIMIT, R_LIMIT, W_LIMIT>
where
    Io: AsyncBufRead + AsyncBufWrite + 'static,
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    D: DateTime,
{
    pub async fn run(
        io: Io,
        addr: SocketAddr,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<H_LIMIT, R_LIMIT, W_LIMIT>,
        service: &'a S,
        date: &'a D,
    ) -> Result<(), Error<S::Error, BE>> {
        let mut dispatcher = Dispatcher::<_, _, _, _, H_LIMIT, R_LIMIT, W_LIMIT> {
            io: SharedIo::new(io),
            timer: Timer::new(timer, config.keep_alive_timeout, config.request_head_timeout),
            ctx: Context::with_addr(addr, date),
            service,
            _phantom: PhantomData,
        };

        let mut read_buf = BytesMut::new();
        let mut write_buf = BytesMut::new();

        loop {
            let (res, r_buf, w_buf) = dispatcher._run(read_buf, write_buf).await;
            read_buf = r_buf;
            write_buf = w_buf;
            if let Err(err) = res {
                handle_error(&mut dispatcher.ctx, &mut write_buf, err)?;
            }

            let (res, w_buf) = write_buf.write(dispatcher.io.io()).await;
            write_buf = w_buf;

            res?;

            if dispatcher.ctx.is_connection_closed() {
                return dispatcher.shutdown().await;
            }
        }
    }

    async fn _run(
        &mut self,
        mut read_buf: BytesMut,
        mut write_buf: BytesMut,
    ) -> (Result<(), Error<S::Error, BE>>, BytesMut, BytesMut) {
        self.timer.update(self.ctx.date().now());

        match read_buf.read(self.io.io()).timeout(self.timer.get()).await {
            Ok((res, r_buf)) => {
                read_buf = r_buf;
                match res {
                    Ok(read) => {
                        if read == 0 {
                            self.ctx.set_close();
                            return (Ok(()), read_buf, write_buf);
                        }
                    }
                    Err(e) => return (Err(e.into()), read_buf, write_buf),
                }
            }
            // read_buf is lost during timeout cancel. make an empty new one instead
            Err(_) => return (Err(self.timer.map_to_err()), BytesMut::new(), write_buf),
        }

        loop {
            let (req, decoder) = match self.ctx.decode_head::<R_LIMIT>(&mut read_buf) {
                Ok(Some(req)) => req,
                Ok(None) => break,
                Err(e) => return (Err(e.into()), read_buf, write_buf),
            };

            self.timer.reset_state();

            let (wait_for_notify, body) = if decoder.is_eof() {
                (false, RequestBody::none())
            } else {
                let body = body(
                    self.io.notifier(),
                    self.ctx.is_expect_header(),
                    R_LIMIT,
                    decoder,
                    read_buf.split(),
                );

                (true, body)
            };

            let req = req.map(|ext| ext.map_body(|_| ReqB::from(body)));

            let (parts, body) = match self.service.call(req).await {
                Ok(res) => res.into_parts(),
                Err(e) => return (Err(Error::Service(e)), read_buf, write_buf),
            };

            let mut encoder = match self.ctx.encode_head(parts, &body, &mut write_buf) {
                Ok(encoder) => encoder,
                Err(e) => return (Err(e.into()), read_buf, write_buf),
            };

            // this block is necessary. ResB has to be dropped asap as it may hold ownership of
            // Body type which if not dropped before Notifier::notify is called would prevent
            // Notifier from waking up Notify.
            {
                let mut body = pin!(body);

                loop {
                    let res = poll_fn(|cx| match body.as_mut().poll_next(cx) {
                        Poll::Ready(res) => Poll::Ready(SelectOutput::A(res)),
                        Poll::Pending if write_buf.is_empty() => Poll::Pending,
                        Poll::Pending => Poll::Ready(SelectOutput::B(())),
                    })
                    .await;

                    match res {
                        SelectOutput::A(Some(Ok(bytes))) => {
                            encoder.encode(bytes, &mut write_buf);
                            if write_buf.len() < W_LIMIT {
                                continue;
                            }
                        }
                        SelectOutput::A(Some(Err(e))) => {
                            let (res, w_buf) = self.on_body_error(e, write_buf).await;
                            write_buf = w_buf;
                            return (res, read_buf, write_buf);
                        }
                        SelectOutput::A(None) => break encoder.encode_eof(&mut write_buf),
                        SelectOutput::B(_) => {}
                    }

                    let (res, w_buf) = write_buf.write(self.io.io()).await;
                    write_buf = w_buf;
                    if let Err(e) = res {
                        return (Err(e.into()), read_buf, write_buf);
                    }
                }
            }

            if wait_for_notify {
                match self.io.wait().await {
                    Some(r_buf) => read_buf = r_buf,
                    None => {
                        self.ctx.set_close();
                        break;
                    }
                }
            }
        }

        (Ok(()), read_buf, write_buf)
    }

    #[cold]
    #[inline(never)]
    async fn shutdown(self) -> Result<(), Error<S::Error, BE>> {
        self.io.io().shutdown(Shutdown::Both).await.map_err(Into::into)
    }

    #[cold]
    #[inline(never)]
    async fn on_body_error(&mut self, e: BE, write_buf: BytesMut) -> (Result<(), Error<S::Error, BE>>, BytesMut) {
        let (res, write_buf) = write_buf.write(self.io.io()).await;
        let e = res.err().map(Error::from).unwrap_or(Error::Body(e));
        (Err(e), write_buf)
    }
}

fn body<Io>(
    io: NotifierIo<Io>,
    is_expect: bool,
    limit: usize,
    decoder: TransferCoding,
    read_buf: BytesMut,
) -> RequestBody
where
    Io: AsyncBufRead + AsyncBufWrite + 'static,
{
    let body = BodyInner {
        io,
        decoder: Decoder {
            decoder,
            limit,
            read_buf,
        },
    };

    let state = if is_expect {
        State::ExpectWrite {
            fut: async {
                let (res, _) = write_all(body.io.io(), CONTINUE_BYTES).await;
                res.map(|_| body)
            },
        }
    } else {
        State::Body { body }
    };

    RequestBody::stream(BodyReader { chunk_read, state })
}

pin_project! {
    #[project = StateProj]
    #[project_replace = StateProjReplace]
    enum State<Io, FutC, FutE> {
        Body {
            body: BodyInner<Io>
        },
        ChunkRead {
            #[pin]
            fut: FutC
        },
        ExpectWrite {
            #[pin]
            fut: FutE,
        },
        None,
    }
}

pin_project! {
    struct BodyReader<Io, F, FutC, FutE> {
        chunk_read: F,
        #[pin]
        state: State<Io, FutC, FutE>
    }
}

struct BodyInner<Io> {
    io: NotifierIo<Io>,
    decoder: Decoder,
}

async fn chunk_read<Io>(mut body: BodyInner<Io>) -> io::Result<(usize, BodyInner<Io>)>
where
    Io: AsyncBufRead,
{
    let (res, r_buf) = body.decoder.read_buf.split().read(body.io.io()).await;
    body.decoder.read_buf.unsplit(r_buf);
    let read = res?;
    Ok((read, body))
}

impl<Io, F, FutC, FutE> Stream for BodyReader<Io, F, FutC, FutE>
where
    Io: AsyncBufRead,
    F: Fn(BodyInner<Io>) -> FutC,
    FutC: Future<Output = io::Result<(usize, BodyInner<Io>)>>,
    FutE: Future<Output = io::Result<BodyInner<Io>>>,
{
    type Item = io::Result<Bytes>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            match this.state.as_mut().project() {
                StateProj::Body { body } => {
                    match body.decoder.decode() {
                        ChunkResult::Ok(bytes) => return Poll::Ready(Some(Ok(bytes))),
                        ChunkResult::Err(e) => return Poll::Ready(Some(Err(e))),
                        ChunkResult::InsufficientData => body.decoder.limit_check()?,
                        _ => return Poll::Ready(None),
                    }

                    let StateProjReplace::Body { body } = this.state.as_mut().project_replace(State::None) else {
                        unreachable!()
                    };
                    this.state.as_mut().project_replace(State::ChunkRead {
                        fut: (this.chunk_read)(body),
                    });
                }
                StateProj::ChunkRead { fut } => {
                    let (read, body) = ready!(fut.poll(cx))?;
                    if read == 0 {
                        this.state.as_mut().project_replace(State::None);
                        return Poll::Ready(None);
                    }
                    this.state.as_mut().project_replace(State::Body { body });
                }
                StateProj::ExpectWrite { fut } => {
                    let body = ready!(fut.poll(cx))?;
                    this.state.as_mut().project_replace(State::ChunkRead {
                        fut: (this.chunk_read)(body),
                    });
                }
                StateProj::None => return Poll::Ready(None),
            }
        }
    }
}

impl<Io> Drop for BodyInner<Io> {
    fn drop(&mut self) {
        if self.decoder.decoder.is_eof() {
            let buf = mem::take(&mut self.decoder.read_buf);
            self.io.notify(buf);
        }
    }
}

struct Decoder {
    decoder: TransferCoding,
    limit: usize,
    read_buf: BytesMut,
}

impl Decoder {
    fn decode(&mut self) -> ChunkResult {
        self.decoder.decode(&mut self.read_buf)
    }

    fn limit_check(&self) -> io::Result<()> {
        if self.read_buf.len() < self.limit {
            return Ok(());
        }

        let msg = format!(
            "READ_BUF_LIMIT reached: {{ limit: {}, length: {} }}",
            self.limit,
            self.read_buf.len()
        );
        Err(io::Error::other(msg))
    }
}

struct SharedIo<Io> {
    inner: Rc<_SharedIo<Io>>,
}

struct _SharedIo<Io> {
    io: Io,
    notify: RefCell<Inner<BytesMut>>,
}

impl<Io> SharedIo<Io> {
    fn new(io: Io) -> Self {
        Self {
            inner: Rc::new(_SharedIo {
                io,
                notify: RefCell::new(Inner { waker: None, val: None }),
            }),
        }
    }

    #[inline(always)]
    fn io(&self) -> &Io {
        &self.inner.io
    }

    fn notifier(&mut self) -> NotifierIo<Io> {
        NotifierIo {
            inner: self.inner.clone(),
        }
    }

    fn wait(&mut self) -> impl Future<Output = Option<BytesMut>> {
        poll_fn(|cx| {
            let mut inner = self.inner.notify.borrow_mut();
            if let Some(val) = inner.val.take() {
                return Poll::Ready(Some(val));
            } else if Rc::strong_count(&self.inner) == 1 {
                return Poll::Ready(None);
            }
            inner.waker = Some(cx.waker().clone());
            Poll::Pending
        })
    }
}

struct NotifierIo<Io> {
    inner: Rc<_SharedIo<Io>>,
}

impl<Io> Drop for NotifierIo<Io> {
    fn drop(&mut self) {
        if let Some(waker) = self.inner.notify.borrow_mut().waker.take() {
            waker.wake();
        }
    }
}

impl<Io> NotifierIo<Io> {
    fn io(&self) -> &Io {
        &self.inner.io
    }

    fn notify(&mut self, val: BytesMut) {
        self.inner.notify.borrow_mut().val = Some(val);
    }
}

struct Inner<V> {
    waker: Option<Waker>,
    val: Option<V>,
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

struct Timer<'a> {
    timer: Pin<&'a mut KeepAlive>,
    state: TimerState,
    ka_dur: Duration,
    req_dur: Duration,
}

impl<'a> Timer<'a> {
    fn new(timer: Pin<&'a mut KeepAlive>, ka_dur: Duration, req_dur: Duration) -> Self {
        Self {
            timer,
            state: TimerState::Idle,
            ka_dur,
            req_dur,
        }
    }

    fn reset_state(&mut self) {
        self.state = TimerState::Idle;
    }

    fn get(&mut self) -> Pin<&mut KeepAlive> {
        self.timer.as_mut()
    }

    // update timer with a given base instant value. the final deadline is calculated base on it.
    fn update(&mut self, now: tokio::time::Instant) {
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
    fn map_to_err<SE, BE>(&self) -> Error<SE, BE> {
        match self.state {
            TimerState::Wait => Error::KeepAliveExpire,
            TimerState::Throttle => Error::RequestTimeout,
            TimerState::Idle => unreachable!(),
        }
    }
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
