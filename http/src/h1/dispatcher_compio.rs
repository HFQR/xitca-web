use core::{
    cell::RefCell,
    future::poll_fn,
    marker::PhantomData,
    mem,
    net::SocketAddr,
    pin::{Pin, pin},
    task::{self, Poll, Waker, ready},
};

use std::{io, rc::Rc};

use compio_buf::{BufResult, IntoInner, IoBuf};
use compio_io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use compio_net::TcpStream;
use futures_core::stream::Stream;
use pin_project_lite::pin_project;
use xitca_service::Service;
use xitca_unsafe_collection::futures::SelectOutput;

use crate::{
    bytes::{Bytes, BytesMut},
    date::DateTime,
    h1::{body::RequestBody, error::Error},
    http::response::Response,
};

use super::{
    dispatcher::handle_error,
    proto::{
        codec::{ChunkResult, TransferCoding},
        context::Context,
        encode::CONTINUE_BYTES,
    },
};

type ExtRequest<B> = crate::http::Request<crate::http::RequestExt<B>>;

/// Http/1 dispatcher
pub struct Dispatcher<'a, S, ReqB, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize> {
    io: SharedIo<TcpStream>,
    ctx: Context<'a, D, H_LIMIT>,
    service: &'a S,
    read_buf: BytesMut,
    write_buf: BytesMut,
    _phantom: PhantomData<ReqB>,
}

trait BufIo {
    async fn read(&mut self, io: &TcpStream) -> io::Result<usize>;

    async fn write(&mut self, io: &TcpStream) -> io::Result<()>;
}

impl BufIo for BytesMut {
    async fn read(&mut self, mut io: &TcpStream) -> io::Result<usize> {
        let mut buf = mem::take(self);

        let len = buf.len();

        if len == buf.capacity() {
            buf.reserve(4096);
        }

        let BufResult(res, buf) = (&mut io).read(buf.slice(len..)).await;
        *self = buf.into_inner();
        res
    }

    async fn write(&mut self, mut io: &TcpStream) -> io::Result<()> {
        let buf = mem::take(self);
        let BufResult(res, mut buf) = (&mut io).write_all(buf).await;
        buf.clear();
        *self = buf;
        res
    }
}

impl<'a, S, ReqB, ResB, BE, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize>
    Dispatcher<'a, S, ReqB, D, H_LIMIT, R_LIMIT, W_LIMIT>
where
    S: Service<ExtRequest<ReqB>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    ResB: Stream<Item = Result<Bytes, BE>>,
    D: DateTime,
{
    pub async fn run(io: TcpStream, addr: SocketAddr, service: &'a S, date: &'a D) -> Result<(), Error<S::Error, BE>> {
        let mut dispatcher = Dispatcher::<_, _, _, H_LIMIT, R_LIMIT, W_LIMIT> {
            io: SharedIo::new(io),
            ctx: Context::with_addr(addr, date),
            service,
            read_buf: BytesMut::new(),
            write_buf: BytesMut::new(),
            _phantom: PhantomData,
        };

        loop {
            if let Err(err) = dispatcher._run().await {
                handle_error(&mut dispatcher.ctx, &mut dispatcher.write_buf, err)?;
            }

            dispatcher.write_buf.write(dispatcher.io.io()).await?;

            if dispatcher.ctx.is_connection_closed() {
                return dispatcher.shutdown().await;
            }
        }
    }

    async fn _run(&mut self) -> Result<(), Error<S::Error, BE>> {
        let read = self.read_buf.read(self.io.io()).await?;

        if read == 0 {
            self.ctx.set_close();
            return Ok(());
        }

        while let Some((req, decoder)) = self.ctx.decode_head::<R_LIMIT>(&mut self.read_buf)? {
            let (wait_for_notify, body) = if decoder.is_eof() {
                (false, RequestBody::none())
            } else {
                let body = body(
                    self.io.notifier(),
                    self.ctx.is_expect_header(),
                    R_LIMIT,
                    decoder,
                    mem::take(&mut self.read_buf),
                );

                (true, body)
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

                    self.write_buf.write(self.io.io()).await?;
                }
            }

            if wait_for_notify {
                match self.io.wait().await {
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
        self.io.take().shutdown().await.map_err(Into::into)
    }

    #[cold]
    #[inline(never)]
    async fn on_body_error(&mut self, e: BE) -> Result<(), Error<S::Error, BE>> {
        self.write_buf.write(self.io.io()).await?;
        Err(Error::Body(e))
    }
}

fn body(
    io: NotifierIo<TcpStream>,
    is_expect: bool,
    limit: usize,
    decoder: TransferCoding,
    read_buf: BytesMut,
) -> RequestBody {
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
                let BufResult(res, _) = body.io.io().write_all(BytesMut::from(CONTINUE_BYTES)).await;
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
    enum State<FutC, FutE> {
        Body {
            body: BodyInner
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
    struct BodyReader< F, FutC, FutE> {
        chunk_read: F,
        #[pin]
        state: State<FutC, FutE>
    }
}

struct BodyInner {
    io: NotifierIo<TcpStream>,
    decoder: Decoder,
}

async fn chunk_read(mut body: BodyInner) -> io::Result<(usize, BodyInner)> {
    let read = body.decoder.read_buf.read(body.io.io()).await?;
    Ok((read, body))
}

impl<F, FutC, FutE> Stream for BodyReader<F, FutC, FutE>
where
    F: Fn(BodyInner) -> FutC,
    FutC: Future<Output = io::Result<(usize, BodyInner)>>,
    FutE: Future<Output = io::Result<BodyInner>>,
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

impl Drop for BodyInner {
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

    fn take(self) -> Io {
        Rc::try_unwrap(self.inner)
            .ok()
            .expect("SharedIo must have exclusive ownership to Io when closing connection")
            .io
    }

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
