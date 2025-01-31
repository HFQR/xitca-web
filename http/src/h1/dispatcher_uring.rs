use core::{
    cell::RefCell,
    fmt,
    future::{Future, poll_fn},
    marker::PhantomData,
    mem,
    net::SocketAddr,
    ops::{Deref, DerefMut},
    pin::{Pin, pin},
    task::{self, Poll, Waker, ready},
};

use std::{io, net::Shutdown, rc::Rc};

use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use tracing::trace;
use xitca_io::{
    bytes::BytesMut,
    io_uring::{AsyncBufRead, AsyncBufWrite, BoundedBuf, write_all},
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
    dispatcher::{Timer, status_only},
    proto::{
        codec::{ChunkResult, TransferCoding},
        context::Context,
        encode::CONTINUE_BYTES,
        error::ProtoError,
    },
};

type ExtRequest<B> = crate::http::Request<crate::http::RequestExt<B>>;

/// Http/1 dispatcher
pub(super) struct Dispatcher<'a, Io, S, ReqB, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize> {
    io: Rc<Io>,
    timer: Timer<'a>,
    ctx: Context<'a, D, H_LIMIT>,
    service: &'a S,
    read_buf: BufOwned,
    write_buf: BufOwned,
    notify: Notify<BufOwned>,
    _phantom: PhantomData<ReqB>,
}

#[derive(Default)]
struct BufOwned {
    buf: Option<BytesMut>,
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
    fn new() -> Self {
        Self {
            buf: Some(BytesMut::new()),
        }
    }

    async fn read_io(&mut self, io: &impl AsyncBufRead) -> io::Result<usize> {
        let mut buf = self.buf.take().unwrap();

        let len = buf.len();
        buf.reserve(4096);

        let (res, buf) = io.read(buf.slice(len..)).await;
        self.buf = Some(buf.into_inner());
        res
    }

    async fn write_io(&mut self, io: &impl AsyncBufWrite) -> io::Result<()> {
        let buf = self.buf.take().unwrap();
        let (res, mut buf) = write_all(io, buf).await;
        buf.clear();
        self.buf = Some(buf);
        res
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
    pub(super) fn new(
        io: Io,
        addr: SocketAddr,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<H_LIMIT, R_LIMIT, W_LIMIT>,
        service: &'a S,
        date: &'a D,
    ) -> Self {
        Self {
            io: Rc::new(io),
            timer: Timer::new(timer, config.keep_alive_timeout, config.request_head_timeout),
            ctx: Context::<_, H_LIMIT>::with_addr(addr, date),
            service,
            read_buf: BufOwned::new(),
            write_buf: BufOwned::new(),
            notify: Notify::new(),
            _phantom: PhantomData,
        }
    }

    pub(super) async fn run(mut self) -> Result<(), Error<S::Error, BE>> {
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
            }

            self.write_buf.write_io(&*self.io).await?;

            if self.ctx.is_connection_closed() {
                return self.io.shutdown(Shutdown::Both).map_err(Into::into);
            }
        }
    }

    async fn _run(&mut self) -> Result<(), Error<S::Error, BE>> {
        self.timer.update(self.ctx.date().now());

        let read = self
            .read_buf
            .read_io(&*self.io)
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
                let body = Body::new(
                    self.io.clone(),
                    self.ctx.is_expect_header(),
                    R_LIMIT,
                    decoder,
                    mem::take(&mut self.read_buf),
                    self.notify.notifier(),
                );

                (Some(&mut self.notify), RequestBody::io_uring(body))
            };

            let req = req.map(|ext| ext.map_body(|_| ReqB::from(body)));

            let (parts, body) = self.service.call(req).await.map_err(Error::Service)?.into_parts();

            let mut encoder = self.ctx.encode_head(parts, &body, &mut *self.write_buf)?;

            // this block is necessary. ResB has to be dropped asap as it may hold ownership of
            // Body type which if not dropped before Notifier::notify is called would prevent
            // Notifier from waking up Notify.
            {
                let mut body = pin!(body);

                loop {
                    let buf = &mut *self.write_buf;

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

                    self.write_buf.write_io(&*self.io).await?;
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
        self.write_buf.write_io(&*self.io).await?;
        Err(Error::Body(e))
    }

    #[cold]
    #[inline(never)]
    fn request_error(&mut self, func: impl FnOnce() -> Response<NoneBody<Bytes>>) {
        self.ctx.set_close();
        let (parts, body) = func().into_parts();
        self.ctx
            .encode_head(parts, &body, &mut *self.write_buf)
            .expect("request_error must be correct");
    }
}

pub(super) struct Body(Pin<Box<dyn Stream<Item = io::Result<Bytes>>>>);

impl Body {
    fn new<Io>(
        io: Rc<Io>,
        is_expect: bool,
        limit: usize,
        decoder: TransferCoding,
        read_buf: BufOwned,
        notify: Notifier<BufOwned>,
    ) -> Self
    where
        Io: AsyncBufRead + AsyncBufWrite + 'static,
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
                    let (res, _) = write_all(&*body.io, CONTINUE_BYTES).await;
                    res.map(|_| body)
                },
            }
        } else {
            State::Body { body }
        };

        Self(Box::pin(BodyReader { chunk_read, state }))
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Body")
    }
}

impl Clone for Body {
    fn clone(&self) -> Self {
        unimplemented!("rework body module so it does not force Clone on Body type.")
    }
}

impl Stream for Body {
    type Item = io::Result<Bytes>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().0).poll_next(cx)
    }
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
    io: Rc<Io>,
    decoder: Decoder,
}

async fn chunk_read<Io>(mut body: BodyInner<Io>) -> io::Result<BodyInner<Io>>
where
    Io: AsyncBufRead,
{
    let read = body.decoder.read_buf.read_io(&*body.io).await?;
    if read == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(body)
}

impl<Io, F, FutC, FutE> Stream for BodyReader<Io, F, FutC, FutE>
where
    Io: AsyncBufRead,
    F: Fn(BodyInner<Io>) -> FutC,
    FutC: Future<Output = io::Result<BodyInner<Io>>>,
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
                        ChunkResult::InsufficientData => {}
                        _ => return Poll::Ready(None),
                    }

                    if body.decoder.read_buf.len() >= body.decoder.limit {
                        let msg = format!(
                            "READ_BUF_LIMIT reached: {{ limit: {}, length: {} }}",
                            body.decoder.limit,
                            body.decoder.read_buf.len()
                        );
                        return Poll::Ready(Some(Err(io::Error::new(io::ErrorKind::Other, msg))));
                    }

                    let StateProjReplace::Body { body } = this.state.as_mut().project_replace(State::None) else {
                        unreachable!()
                    };
                    this.state.as_mut().project_replace(State::ChunkRead {
                        fut: (this.chunk_read)(body),
                    });
                }
                StateProj::ChunkRead { fut } => {
                    let body = ready!(fut.poll(cx))?;
                    this.state.as_mut().project_replace(State::Body { body });
                }
                StateProj::ExpectWrite { fut } => {
                    let body = ready!(fut.poll(cx))?;
                    this.state.as_mut().project_replace(State::ChunkRead {
                        fut: (this.chunk_read)(body),
                    });
                }
                StateProj::None => unreachable!(
                    "None variant is only used internally and must not be observable from stream consumer."
                ),
            }
        }
    }
}

struct Decoder {
    decoder: TransferCoding,
    limit: usize,
    read_buf: BufOwned,
    notify: Notifier<BufOwned>,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if self.decoder.is_eof() {
            let buf = mem::take(&mut self.read_buf);
            self.notify.notify(buf);
        }
    }
}

struct Notify<T>(Rc<RefCell<Inner<T>>>);

impl<T> Notify<T> {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(Inner { waker: None, val: None })))
    }

    fn notifier(&mut self) -> Notifier<T> {
        Notifier(self.0.clone())
    }

    fn wait(&mut self) -> impl Future<Output = Option<T>> + '_ {
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

struct Notifier<T>(Rc<RefCell<Inner<T>>>);

impl<T> Drop for Notifier<T> {
    fn drop(&mut self) {
        if let Some(waker) = self.0.borrow_mut().waker.take() {
            waker.wake();
        }
    }
}

impl<T> Notifier<T> {
    fn notify(&mut self, val: T) {
        self.0.borrow_mut().val = Some(val);
    }
}

struct Inner<V> {
    waker: Option<Waker>,
    val: Option<V>,
}
