use core::{
    cell::RefCell,
    future::poll_fn,
    marker::PhantomData,
    mem,
    net::SocketAddr,
    pin::{Pin, pin},
    task::{self, Poll, Waker, ready},
};

use std::{io, net::Shutdown, rc::Rc};

use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use xitca_io::io_uring::{AsyncBufRead, AsyncBufWrite, BoundedBuf, write_all};
use xitca_service::Service;
use xitca_unsafe_collection::futures::SelectOutput;

use crate::{
    bytes::{Bytes, BytesMut},
    config::HttpServiceConfig,
    date::DateTime,
    h1::{body::RequestBody, error::Error},
    http::response::Response,
    util::timer::{KeepAlive, Timeout},
};

use super::{
    dispatcher::{Timer, handle_error},
    proto::{
        codec::{ChunkResult, TransferCoding},
        context::Context,
        encode::CONTINUE_BYTES,
    },
};

type ExtRequest<B> = crate::http::Request<crate::http::RequestExt<B>>;

/// Http/1 dispatcher
pub(super) struct Dispatcher<'a, Io, S, ReqB, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize> {
    io: Rc<Io>,
    timer: Timer<'a>,
    ctx: Context<'a, D, H_LIMIT>,
    service: &'a S,
    read_buf: BytesMut,
    write_buf: BytesMut,
    notify: Notify<BytesMut>,
    _phantom: PhantomData<ReqB>,
}

trait BufIo {
    async fn read(&mut self, io: &impl AsyncBufRead) -> io::Result<usize>;

    async fn write(&mut self, io: &impl AsyncBufWrite) -> io::Result<()>;
}

impl BufIo for BytesMut {
    async fn read(&mut self, io: &impl AsyncBufRead) -> io::Result<usize> {
        let mut buf = mem::take(self);

        let len = buf.len();
        buf.reserve(4096);

        let (res, buf) = io.read(buf.slice(len..)).await;
        *self = buf.into_inner();
        res
    }

    async fn write(&mut self, io: &impl AsyncBufWrite) -> io::Result<()> {
        let buf = mem::take(self);
        let (res, mut buf) = write_all(io, buf).await;
        buf.clear();
        *self = buf;
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
    pub(super) async fn run(
        io: Io,
        addr: SocketAddr,
        timer: Pin<&'a mut KeepAlive>,
        config: HttpServiceConfig<H_LIMIT, R_LIMIT, W_LIMIT>,
        service: &'a S,
        date: &'a D,
    ) -> Result<(), Error<S::Error, BE>> {
        let mut dispatcher = Dispatcher::<_, _, _, _, H_LIMIT, R_LIMIT, W_LIMIT> {
            io: Rc::new(io),
            timer: Timer::new(timer, config.keep_alive_timeout, config.request_head_timeout),
            ctx: Context::with_addr(addr, date),
            service,
            read_buf: BytesMut::new(),
            write_buf: BytesMut::new(),
            notify: Notify::new(),
            _phantom: PhantomData,
        };

        loop {
            if let Err(err) = dispatcher._run().await {
                handle_error(&mut dispatcher.ctx, &mut dispatcher.write_buf, err)?;
            }

            dispatcher.write_buf.write(&*dispatcher.io).await?;

            if dispatcher.ctx.is_connection_closed() {
                return dispatcher.shutdown();
            }
        }
    }

    async fn _run(&mut self) -> Result<(), Error<S::Error, BE>> {
        self.timer.update(self.ctx.date().now());

        let read = self
            .read_buf
            .read(&*self.io)
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
                (None, RequestBody::none())
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
                    let res = poll_fn(|cx| match body.as_mut().poll_next(cx) {
                        Poll::Ready(res) => Poll::Ready(SelectOutput::A(res)),
                        Poll::Pending if self.write_buf.is_empty() => Poll::Pending,
                        Poll::Pending => Poll::Ready(SelectOutput::B(())),
                    })
                    .await;

                    match res {
                        SelectOutput::A(Some(Ok(bytes))) => {
                            encoder.encode(bytes, &mut self.write_buf);
                            if self.write_buf.len() < W_LIMIT {
                                continue;
                            }
                        }
                        SelectOutput::A(Some(Err(e))) => return self.on_body_error(e).await,
                        SelectOutput::A(None) => break encoder.encode_eof(&mut self.write_buf),
                        SelectOutput::B(_) => {}
                    }

                    self.write_buf.write(&*self.io).await?;
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
    fn shutdown(self) -> Result<(), Error<S::Error, BE>> {
        Rc::try_unwrap(self.io)
            .ok()
            .expect("Dispatcher must have exclusive ownership to Io when closing connection")
            .shutdown(Shutdown::Both)
            .map_err(Into::into)
    }

    #[cold]
    #[inline(never)]
    async fn on_body_error(&mut self, e: BE) -> Result<(), Error<S::Error, BE>> {
        self.write_buf.write(&*self.io).await?;
        Err(Error::Body(e))
    }
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
    io: Rc<Io>,
    decoder: Decoder,
}

async fn chunk_read<Io>(mut body: BodyInner<Io>) -> io::Result<(usize, BodyInner<Io>)>
where
    Io: AsyncBufRead,
{
    let read = body.decoder.read_buf.read(&*body.io).await?;
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

struct Decoder {
    decoder: TransferCoding,
    limit: usize,
    read_buf: BytesMut,
    notify: Notifier<BytesMut>,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if self.decoder.is_eof() {
            let buf = mem::take(&mut self.read_buf);
            self.notify.notify(buf);
        }
    }
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
