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
use tokio_uring::{buf::IoBuf, net::TcpStream};
use tracing::trace;
use xitca_io::bytes::BytesMut;
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
        buffered,
        timer::{KeepAlive, Timeout},
    },
};

use super::{
    dispatcher::{status_only, Timer},
    proto::{
        codec::{ChunkResult, TransferCoding},
        context::Context,
        error::ProtoError,
    },
};

type ExtRequest<B> = crate::http::Request<crate::http::RequestExt<B>>;

/// Http/1 dispatcher
pub(super) struct Dispatcher<'a, S, ReqB, D, const H_LIMIT: usize, const R_LIMIT: usize, const W_LIMIT: usize> {
    io: Rc<TcpStream>,
    timer: Timer<'a>,
    ctx: Context<'a, D, H_LIMIT>,
    service: &'a S,
    read_buf: ReadBuf<R_LIMIT>,
    write_buf: WriteBuf<W_LIMIT>,
    waiter: Waiter<ReadBufErased>,
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

    async fn write_io(&mut self, io: &TcpStream) -> io::Result<()> {
        let mut buf = self
            .buf
            .take()
            .expect("WriteBuf::write_io is dropped before polling to complete");

        if !buf.is_empty() {
            let (res, b) = io.write_all(buf).await;
            buf = b;
            buf.clear();
            res?;
        }

        self.buf.replace(buf);
        Ok(())
    }
}

#[derive(Debug, Default)]
struct ReadBuf<const LIMIT: usize> {
    buf: buffered::ReadBuf<LIMIT>,
}

// erase const generic type to ease public type param.
type ReadBufErased = ReadBuf<0>;

impl<const LIMIT: usize> ReadBuf<LIMIT> {
    fn new() -> Self {
        Self {
            buf: buffered::ReadBuf::new(),
        }
    }

    fn cast_limit<const LIMIT2: usize>(self) -> ReadBuf<LIMIT2> {
        ReadBuf {
            buf: self.buf.cast_limit(),
        }
    }

    fn len(&self) -> usize {
        self.buf.len()
    }

    async fn read_io(&mut self, io: &TcpStream) -> io::Result<()> {
        let mut buf = mem::take(&mut self.buf).into_inner().into_inner();

        let len = buf.len();
        let remaining = buf.capacity() - len;
        if remaining < 4096 {
            buf.reserve(4096 - remaining);
        }

        let (res, buf) = io.read(buf.slice(len..)).await;
        let n = res?;

        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        self.buf = buffered::ReadBuf::from(buf.into_inner());

        Ok(())
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
    pub(super) fn new(
        io: TcpStream,
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
            waiter: Waiter::new(),
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

            self.write_buf.write_io(&self.io).await?;

            if self.ctx.is_connection_closed() {
                return self.io.shutdown(Shutdown::Both).map_err(Into::into);
            }
        }
    }

    async fn _run(&mut self) -> Result<(), Error<S::Error, BE>> {
        self.timer.update(self.ctx.date().now());

        self.read_buf
            .read_io(&self.io)
            .timeout(self.timer.get())
            .await
            .map_err(|_| self.timer.map_to_err())??;

        while let Some((req, decoder)) = self.ctx.decode_head::<R_LIMIT>(&mut self.read_buf.buf)? {
            self.timer.reset_state();

            let (waiter, body) = if decoder.is_eof() {
                (None, RequestBody::default())
            } else {
                let notify = self.waiter.notifier();

                let body = Body::new(
                    self.io.clone(),
                    R_LIMIT,
                    decoder,
                    mem::take(&mut self.read_buf).cast_limit(),
                    notify,
                );

                (Some(&mut self.waiter), RequestBody::io_uring(body))
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
                            let buf = self.write_buf.get_mut().split();
                            task.as_mut().set(Some(self.io.write_all(buf)));
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
                        SelectOutput::B((res, _)) => {
                            task.as_mut().set(None);
                            res?
                        }
                    }
                }

                if let Some(task) = task.as_pin_mut() {
                    let (res, _) = task.await;
                    res?;
                }

                if let Some(e) = err {
                    return Err(e);
                }
            }

            if let Some(waiter) = waiter {
                match waiter.wait().await {
                    Some(read_buf) => {
                        let _ = mem::replace(&mut self.read_buf, read_buf.cast_limit());
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
    io: Rc<TcpStream>,
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
        io: Rc<TcpStream>,
        limit: usize,
        decoder: TransferCoding,
        read_buf: ReadBufErased,
        notify: Notifier<ReadBufErased>,
    ) -> Self {
        Self::_Body(_Body {
            io,
            limit,
            decoder: Decoder {
                decoder,
                read_buf,
                notify,
            },
        })
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
                    match body.decoder.decoder.decode(&mut body.decoder.read_buf.buf) {
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
                        body.decoder.read_buf.read_io(&body.io).await.map(|_| body)
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

struct Waiter<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Waiter<T> {
    fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner { waker: None, val: None })),
        }
    }

    fn notifier(&mut self) -> Notifier<T> {
        let mut inner = self.inner.borrow_mut();
        inner.val = None;
        inner.waker = None;
        Notifier {
            inner: self.inner.clone(),
            notified: false,
        }
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

struct Notifier<T> {
    inner: Rc<RefCell<Inner<T>>>,
    notified: bool,
}

impl<T> Drop for Notifier<T> {
    fn drop(&mut self) {
        if !self.notified {
            if let Some(waker) = self.inner.borrow_mut().waker.take() {
                waker.wake();
            }
        }
    }
}

impl<T> Notifier<T> {
    fn notify(&mut self, val: T) {
        self.notified = true;
        let mut inner = self.inner.borrow_mut();
        inner.val = Some(val);
        if let Some(waker) = inner.waker.take() {
            waker.wake();
        }
    }
}

struct Inner<V> {
    waker: Option<Waker>,
    val: Option<V>,
}
