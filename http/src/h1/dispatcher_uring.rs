use core::{
    cell::RefCell,
    fmt,
    future::{pending, poll_fn, Future},
    marker::PhantomData,
    mem,
    pin::{pin, Pin},
    task::{ready, Poll, Waker},
};

use std::{
    io,
    net::{Shutdown, SocketAddr},
    rc::Rc,
};

use futures_core::stream::Stream;
use tracing::trace;
use xitca_io::{
    bytes::BytesMut,
    io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf, Slice},
};
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{
    body::NoneBody,
    bytes::Bytes,
    config::HttpServiceConfig,
    date::DateTime,
    h1::{body::RequestBody, error::Error},
    http::{response::Response, StatusCode},
    util::{
        buffered::ReadBuf,
        timer::{KeepAlive, Timeout},
    },
};

use super::{
    dispatcher::{status_only, Timer},
    proto::{
        codec::{ChunkResult, TransferCoding},
        context::Context,
        encode::encode_continue,
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
    read_buf: ReadBuf<R_LIMIT>,
    write_buf: WriteBuf<W_LIMIT>,
    notify: Notify<ReadBufErased>,
    _phantom: PhantomData<ReqB>,
}

struct WriteBuf<const LIMIT: usize> {
    buf: Option<BytesMut>,
}

impl<const LIMIT: usize> WriteBuf<LIMIT> {
    fn new() -> Self {
        Self {
            buf: Some(BytesMut::new()),
        }
    }

    fn get_mut(&mut self) -> &mut BytesMut {
        self.buf
            .as_mut()
            .expect("WriteBuf::write_io is dropped before polling to complete")
    }

    async fn write_io(&mut self, io: &impl AsyncBufWrite) -> io::Result<()> {
        let buf = self
            .buf
            .take()
            .expect("WriteBuf::write_io is dropped before polling to complete");

        let (res, mut buf) = write_all_with(buf, |slice| io.write(slice)).await;

        buf.clear();
        self.buf.replace(buf);

        res
    }

    fn split_write_io<'i>(&mut self, io: &'i impl AsyncBufWrite) -> impl Future<Output = io::Result<()>> + 'i {
        let buf = self.get_mut().split();
        async {
            let (res, _) = write_all_with(buf, |slice| io.write(slice)).await;
            res
        }
    }
}

async fn write_all_with<F, Fut>(mut buf: BytesMut, mut func: F) -> (io::Result<()>, BytesMut)
where
    F: FnMut(Slice<BytesMut>) -> Fut,
    Fut: Future<Output = (io::Result<usize>, Slice<BytesMut>)>,
{
    let mut n = 0;
    while n < buf.bytes_init() {
        match func(buf.slice(n..)).await {
            (Ok(0), slice) => {
                return (Err(io::ErrorKind::WriteZero.into()), slice.into_inner());
            }
            (Ok(m), slice) => {
                n += m;
                buf = slice.into_inner();
            }
            (Err(e), slice) => {
                return (Err(e), slice.into_inner());
            }
        }
    }
    (Ok(()), buf)
}

// erase const generic type to ease public type param.
type ReadBufErased = ReadBuf<0>;

impl<const LIMIT: usize> ReadBuf<LIMIT> {
    async fn read_with<F, Fut>(&mut self, func: F) -> io::Result<usize>
    where
        F: FnOnce(Slice<BytesMut>) -> Fut,
        Fut: Future<Output = (io::Result<usize>, Slice<BytesMut>)>,
    {
        let mut buf = mem::take(self).into_inner().into_inner();

        let len = buf.len();
        let remaining = buf.capacity() - len;
        if remaining < 4096 {
            buf.reserve(4096 - remaining);
        }

        let (res, buf) = func(buf.slice(len..)).await;
        *self = Self::from(buf.into_inner());
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
            read_buf: ReadBuf::<R_LIMIT>::new(),
            write_buf: WriteBuf::<W_LIMIT>::new(),
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
            .read_with(|slice| self.io.read(slice))
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
                    mem::take(&mut self.read_buf).limit(),
                    self.notify.notifier(),
                );

                (Some(&mut self.notify), RequestBody::io_uring(body))
            };

            let req = req.map(|ext| ext.map_body(|_| ReqB::from(body)));

            let (parts, body) = self.service.call(req).await.map_err(Error::Service)?.into_parts();

            let mut encoder = self.ctx.encode_head(parts, &body, self.write_buf.get_mut())?;

            // this block is necessary. ResB has to be dropped asap as it may hold ownership of
            // Body type which if not dropped before Waiter::wait is called would prevent Notifier
            // from waking up Waiter.
            {
                let mut body = pin!(body);
                let mut task = pin!(None);
                let mut err = None;

                loop {
                    let want_body = self.write_buf.get_mut().len() < W_LIMIT;

                    let poll_body = async {
                        if want_body {
                            poll_fn(|cx| body.as_mut().poll_next(cx)).await
                        } else {
                            pending().await
                        }
                    };

                    let poll_write = async {
                        if task.is_none() {
                            if self.write_buf.get_mut().is_empty() {
                                // pending when buffer is empty. wait for body to make progress
                                // with more bytes. (or exit with error)
                                return pending().await;
                            }
                            task.as_mut().set(Some(self.write_buf.split_write_io(&*self.io)));
                        }
                        task.as_mut().as_pin_mut().unwrap().await
                    };

                    match poll_body.select(poll_write).await {
                        SelectOutput::A(Some(Ok(bytes))) => {
                            encoder.encode(bytes, self.write_buf.get_mut());
                        }
                        SelectOutput::A(None) => {
                            encoder.encode_eof(self.write_buf.get_mut());
                            break;
                        }
                        SelectOutput::A(Some(Err(e))) => {
                            err = Some(Error::Body(e));
                            break;
                        }
                        SelectOutput::B(res) => {
                            task.as_mut().set(None);
                            res?
                        }
                    }
                }

                if let Some(task) = task.as_pin_mut() {
                    task.await?;
                }

                if let Some(e) = err {
                    return Err(e);
                }
            }

            if let Some(waiter) = waiter {
                match waiter.wait().await {
                    Some(read_buf) => {
                        let _ = mem::replace(&mut self.read_buf, read_buf.limit());
                    }
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
    fn request_error(&mut self, func: impl FnOnce() -> Response<NoneBody<Bytes>>) {
        self.ctx.set_close();
        let (parts, body) = func().into_parts();
        self.ctx
            .encode_head(parts, &body, self.write_buf.get_mut())
            .expect("request_error must be correct");
    }
}

pub(super) enum Body {
    _Body(_Body),
    _Future(BodyFuture),
    None,
}

pub(super) type BodyFuture = Pin<Box<dyn Future<Output = io::Result<_Body>>>>;

pub(super) struct _Body {
    io: Rc<dyn AsyncBufReadH1Body>,
    limit: usize,
    decoder: Decoder,
}

struct Decoder {
    decoder: TransferCoding,
    read_buf: ReadBufErased,
    notify: Notifier<ReadBufErased>,
}

impl Body {
    fn new(
        io: Rc<dyn AsyncBufReadH1Body>,
        is_expect: bool,
        limit: usize,
        decoder: TransferCoding,
        read_buf: ReadBufErased,
        notify: Notifier<ReadBufErased>,
    ) -> Self {
        let body = _Body {
            io,
            limit,
            decoder: Decoder {
                decoder,
                read_buf,
                notify,
            },
        };

        if is_expect {
            Self::_Future(Box::pin(async {
                let mut bytes = BytesMut::new();
                encode_continue(&mut bytes);
                let (res, _) = write_all_with(bytes, |slice| body.io.write_dyn(slice)).await;
                res.map(|_| body)
            }))
        } else {
            Self::_Body(body)
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

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match self.as_mut().get_mut() {
                Self::_Body(body) => {
                    match body.decoder.decoder.decode(&mut body.decoder.read_buf) {
                        ChunkResult::Ok(bytes) => return Poll::Ready(Some(Ok(bytes))),
                        ChunkResult::Err(e) => return Poll::Ready(Some(Err(e))),
                        ChunkResult::InsufficientData => {}
                        _ => return Poll::Ready(None),
                    }

                    if body.decoder.read_buf.len() >= body.limit {
                        let msg = format!(
                            "READ_BUF_LIMIT reached: {{ limit: {}, length: {} }}",
                            body.limit,
                            body.decoder.read_buf.len()
                        );
                        return Poll::Ready(Some(Err(io::Error::new(io::ErrorKind::Other, msg))));
                    }

                    let Self::_Body(mut body) = mem::replace(self.as_mut().get_mut(), Self::None) else { unreachable!() };

                    self.as_mut().set(Self::_Future(Box::pin(async {
                        let read = body.decoder.read_buf.read_with(|slice| body.io.read_dyn(slice)).await?;
                        if read == 0 {
                            return Err(io::ErrorKind::UnexpectedEof.into());
                        }
                        Ok(body)
                    })))
                }
                Self::_Future(fut) => {
                    let body = ready!(Pin::new(fut).poll(cx))?;
                    self.as_mut().set(Body::_Body(body));
                }
                Self::None => unreachable!(
                    "None variant is only used internally and must not be observable from stream consumer."
                ),
            }
        }
    }
}

struct Notify<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Notify<T> {
    fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner { waker: None, val: None })),
        }
    }

    fn notifier(&mut self) -> Notifier<T> {
        Notifier(self.inner.clone())
    }

    async fn wait(&mut self) -> Option<T> {
        poll_fn(|cx| {
            let strong_count = Rc::strong_count(&self.inner);
            let mut inner = self.inner.borrow_mut();
            if let Some(val) = inner.val.take() {
                return Poll::Ready(Some(val));
            } else if strong_count == 1 {
                return Poll::Ready(None);
            }
            inner.waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
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

type Output<'s> = Pin<Box<dyn Future<Output = (io::Result<usize>, Slice<BytesMut>)> + 's>>;

trait AsyncBufReadH1Body {
    fn read_dyn(&self, buf: Slice<BytesMut>) -> Output;

    fn write_dyn(&self, buf: Slice<BytesMut>) -> Output;
}

impl<Io> AsyncBufReadH1Body for Io
where
    Io: AsyncBufRead + AsyncBufWrite,
{
    fn read_dyn(&self, buf: Slice<BytesMut>) -> Output {
        Box::pin(<Io as AsyncBufRead>::read(self, buf))
    }

    fn write_dyn(&self, buf: Slice<BytesMut>) -> Output {
        Box::pin(<Io as AsyncBufWrite>::write(self, buf))
    }
}