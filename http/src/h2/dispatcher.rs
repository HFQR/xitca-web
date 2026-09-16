use core::{
    cell::RefCell,
    fmt,
    future::poll_fn,
    net::SocketAddr,
    pin::{Pin, pin},
    task::{Context, Poll, ready},
    time::Duration,
};

use std::{io, net::Shutdown, rc::Rc};

use pin_project_lite::pin_project;
use tracing::error;
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncBufRead, AsyncBufWrite, BoundedBuf},
};
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::{
    body::{Body, SizeHint},
    bytes::Bytes,
    config::HttpServiceConfig,
    date::{DateTime, DateTimeHandle},
    http::{
        Extension, HeaderMap, Method, Request, RequestExt, Response, Version,
        header::{CONTENT_LENGTH, DATE},
    },
    util::{
        futures::Queue,
        timer::{KeepAlive, Timeout},
    },
};

use super::{
    body::RequestBody,
    proto::{
        codec::DecodeContext,
        flow::{FlowControl, FlowControlClone, FlowControlLock, Frame},
        frame::{PREFACE, headers, settings, stream_id::StreamId},
        ping_pong::PingPong,
    },
};

struct Decoder<'a, S> {
    ctx: DecodeContext,
    flow: &'a FlowControlClone,
    service: &'a S,
    date: &'a DateTimeHandle,
    addr: SocketAddr,
}

impl<'a, S> Decoder<'a, S> {
    fn new(
        flow: &'a FlowControlClone,
        service: &'a S,
        max_frame_size: usize,
        max_header_list_size: usize,
        addr: SocketAddr,
        date: &'a DateTimeHandle,
    ) -> Self {
        Self {
            ctx: DecodeContext::new(max_frame_size, max_header_list_size),
            flow,
            service,
            date,
            addr,
        }
    }
}

async fn read_io<const LIMIT: usize>(mut buf: BytesMut, io: &impl AsyncBufRead) -> (io::Result<usize>, BytesMut) {
    if buf.len() >= LIMIT {
        // Unprocessed data has hit the cap. Yield without issuing a new read
        // until the caller drains the buffer and restarts the task.
        return core::future::pending().await;
    }
    let len = buf.len();
    buf.reserve(4096);
    let (res, buf) = io.read(buf.slice(len..)).await;
    (res, buf.into_inner())
}

async fn response_task<S, ReqB, ResB, ResBE>(
    req: Request<RequestExt<RequestBody>>,
    stream_id: StreamId,
    service: &S,
    ctx: &FlowControlLock,
    date: &DateTimeHandle,
) -> Result<(), ()>
where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ReqB: From<RequestBody>,
    ResB: Body<Data = Bytes, Error = ResBE>,
    ResBE: fmt::Debug,
{
    // the response body is dropped with the future when _response_task returns, so
    // no FlowControl borrow is alive when RequestBody::drop re-enters the RefCell.
    let res = _response_task(req, stream_id, service, ctx, date).await;

    let mut flow = ctx.borrow_mut();

    match res {
        Ok(SendOutcome::Finished) => {}
        Ok(SendOutcome::EndStream) => flow.send_end_stream(stream_id),
        Ok(SendOutcome::Trailers(trailers)) => flow.send_trailers(stream_id, trailers),
        Err(()) => flow.internal_reset(&stream_id),
    }

    flow.response_task_done(stream_id)
}

async fn _response_task<S, ReqB, ResB, ResBE>(
    req: Request<RequestExt<RequestBody>>,
    stream_id: StreamId,
    service: &S,
    flow: &FlowControlLock,
    date: &DateTimeHandle,
) -> Result<SendOutcome, ()>
where
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    S::Error: fmt::Debug,
    ReqB: From<RequestBody>,
    ResB: Body<Data = Bytes, Error = ResBE>,
    ResBE: fmt::Debug,
{
    let req = req.map(|ext| ext.map_body(From::from));

    let head_method = req.method() == Method::HEAD;

    let res = service.call(req).await.map_err(|_| ())?;

    let (mut parts, body) = res.into_parts();

    super::strip_connection_headers::<false>(&mut parts.headers);

    parts
        .headers
        .entry(DATE)
        .or_insert_with(|| date.with_date_header(Clone::clone));

    let end_stream = match (body.size_hint(), head_method) {
        (SizeHint::None, _) => true,
        (SizeHint::Exact(size), is_head) => {
            parts.headers.entry(CONTENT_LENGTH).or_insert_with(|| size.into());
            is_head
        }
        (SizeHint::Unknown, is_head) => is_head,
    };

    let pseudo = headers::Pseudo::response(parts.status);
    let mut headers = headers::Headers::new(stream_id, pseudo, parts.headers);

    if end_stream {
        headers.set_end_stream();
    }

    with_flow(flow, |flow| flow.send_headers(headers));

    if !end_stream {
        SendResponse {
            flow,
            stream_id,
            body,
            state: State::PollBody,
        }
        .await
    } else {
        Ok(SendOutcome::Finished)
    }
}

pin_project! {
    struct SendResponse<'a, ResB> {
        flow: &'a FlowControlLock,
        stream_id: StreamId,
        #[pin]
        body: ResB,
        state: State,
    }
}

enum State {
    PollBody,
    SendData { data: Bytes },
}

enum SendOutcome {
    Finished,
    EndStream,
    Trailers(HeaderMap),
}

#[inline]
fn with_flow<T>(flow: &FlowControlLock, f: impl FnOnce(&mut FlowControl) -> T) -> T {
    f(&mut flow.borrow_mut())
}

impl<ResB, ResBE> Future for SendResponse<'_, ResB>
where
    ResB: Body<Data = Bytes, Error = ResBE>,
    ResBE: fmt::Debug,
{
    type Output = Result<SendOutcome, ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        let flow: &FlowControlLock = this.flow;
        let stream_id = *this.stream_id;

        loop {
            match this.state {
                State::PollBody => match ready!(this.body.as_mut().poll_frame(cx)) {
                    None => return Poll::Ready(Ok(SendOutcome::EndStream)),
                    Some(Err(e)) => {
                        error!("body error: {e:?}");
                        return Poll::Ready(Err(()));
                    }
                    Some(Ok(Frame::Data(data))) => {
                        *this.state = State::SendData { data };
                    }
                    Some(Ok(Frame::Trailers(trailers))) => {
                        return Poll::Ready(Ok(SendOutcome::Trailers(trailers)));
                    }
                },
                State::SendData { data } => {
                    let end_stream = this.body.is_end_stream();

                    let opt = ready!(with_flow(flow, |flow| {
                        flow.poll_send_data(stream_id, data, end_stream, cx)
                    }));

                    // END_STREAM rode out on the last DATA frame.
                    if opt.is_some() {
                        return Poll::Ready(Ok(SendOutcome::Finished));
                    }

                    *this.state = State::PollBody;
                }
            }
        }
    }
}

/// Peek into the given buffer (and read more if needed) to determine whether
/// the connection speaks HTTP/2.  Returns `(version, buf)` where `buf` contains
/// all bytes read so far (unconsumed) so the caller can forward them to the
/// chosen dispatcher.
pub(crate) async fn peek_version(
    io: &(impl AsyncBufRead + AsyncBufWrite),
    buf: BytesMut,
) -> io::Result<(Version, BytesMut)> {
    let (read_buf, res) = prefix_check::<4096>(buf, io).await;
    let version = if res.is_ok() { Version::HTTP_2 } else { Version::HTTP_11 };
    Ok((version, read_buf))
}

pub(crate) async fn run<
    Io,
    S,
    ReqB,
    ResB,
    ResBE,
    const HEADER_LIMIT: usize,
    const READ_BUF_LIMIT: usize,
    const WRITE_BUF_LIMIT: usize,
>(
    io: Io,
    addr: SocketAddr,
    read_buf: BytesMut,
    mut ka: Pin<&mut KeepAlive>,
    service: &S,
    date: &DateTimeHandle,
    config: &HttpServiceConfig<HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>,
) -> io::Result<()>
where
    Io: AsyncBufRead + AsyncBufWrite,
    S: Service<Request<RequestExt<ReqB>>, Response = Response<ResB>>,
    ReqB: From<RequestBody>,
    S::Error: fmt::Debug,
    ResB: Body<Data = Bytes, Error = ResBE>,
    ResBE: fmt::Debug,
{
    let mut settings = settings::Settings::default();

    settings.set_max_concurrent_streams(Some(config.h2_max_concurrent_streams));
    settings.set_initial_window_size(Some(config.h2_initial_window_size));
    settings.set_max_frame_size(Some(config.h2_max_frame_size));
    settings.set_max_header_list_size(Some(config.h2_max_header_list_size));
    settings.set_enable_connect_protocol(Some(1));

    let max_frame_size = config.h2_max_frame_size as usize;
    let max_header_list_size = config.h2_max_header_list_size as usize;

    let mut read_buf = handshake::<READ_BUF_LIMIT>(&io, read_buf, ka.as_mut()).await?;
    let mut write_buf = BytesMut::new();

    let flow = FlowControl::new(&settings);
    let flow = Rc::new(RefCell::new(flow));

    let mut ctx = Decoder::new(&flow, service, max_frame_size, max_header_list_size, addr, date);

    let mut queue = Queue::new();
    let mut ping_pong = PingPong::new(ka.as_mut(), &flow, date, config.keep_alive_timeout);

    flow.borrow_mut().init(settings);

    let res = {
        let mut read_task = pin!(read_io::<READ_BUF_LIMIT>(read_buf, &io));

        let mut write_task = pin!(async {
            while poll_fn(|cx| flow.borrow_mut().poll_encode(&mut write_buf, cx)).await {
                let (res, buf) = io.write(write_buf).await;

                write_buf = buf;

                match res {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => write_buf.advance(n),
                    Err(e) => return Err(e),
                }
            }

            Ok(())
        });

        let shutdown = 'body: loop {
            match read_task
                .as_mut()
                .select(async {
                    while let Ok::<_, ()>(()) = queue.next().await {}
                    // error case means connection going away. enter shutdown
                })
                .select(write_task.as_mut())
                .select(ping_pong.tick())
                .await
            {
                SelectOutput::A(SelectOutput::A(SelectOutput::A((res, buf)))) => {
                    read_buf = buf;

                    match res {
                        Ok(n) if n > 0 => {
                            let flow = &mut *ctx.flow.borrow_mut();
                            loop {
                                match ctx.ctx.try_decode(&mut read_buf, flow) {
                                    Ok(Some((req, id))) => {
                                        let req = req.map(|(size, protocol)| {
                                            let body = RequestBody::new(id, size, ctx.flow.clone());
                                            let ext = Extension::with_protocol(ctx.addr, protocol);
                                            RequestExt::from_parts(body, ext)
                                        });
                                        queue.push(response_task(req, id, ctx.service, ctx.flow, ctx.date));
                                    }
                                    Ok(None) => break,
                                    Err(err) => {
                                        if flow.go_away(err) {
                                            break 'body ShutDown::ReadClosed(Ok(()));
                                        }
                                    }
                                }
                            }
                        }
                        res => break ShutDown::ReadClosed(res.map(|_| ())),
                    };

                    read_task.set(read_io(read_buf, &io));
                }
                SelectOutput::A(SelectOutput::A(SelectOutput::B(_))) => break ShutDown::ReadClosed(Ok(())),
                SelectOutput::A(SelectOutput::B(res)) => break ShutDown::WriteClosed(res),
                SelectOutput::B(Err(e)) => break ShutDown::Timeout(e),
                SelectOutput::B(Ok(_)) => {}
            }
        };

        Box::pin(async {
            let (io_res, want_write) = match shutdown {
                ShutDown::WriteClosed(res) => (res, false),
                ShutDown::Timeout(err) => return Err(err),
                ShutDown::ReadClosed(res) => (res, true),
            };

            ctx.flow.borrow_mut().reset_all_stream(&io_res);

            loop {
                if queue.is_empty() {
                    ctx.flow.borrow_mut().close_write_queue();

                    if !want_write {
                        break io_res;
                    }
                }

                match queue
                    .next()
                    .select(async {
                        if want_write {
                            write_task.as_mut().await
                        } else {
                            core::future::pending().await
                        }
                    })
                    .select(ping_pong.tick())
                    .await
                {
                    SelectOutput::A(SelectOutput::A(_)) => {
                        // response_task already ran response_task_done before
                        // returning. An Err(()) here means a second GoAway was
                        // escalated — moot, we're already draining.
                    }
                    SelectOutput::A(SelectOutput::B(res)) => {
                        res?;
                        break io_res;
                    }
                    SelectOutput::B(res) => res?,
                }
            }
        })
        .await
    };

    lingering_read(&io, ka, date).await?;

    // Send FIN so the peer sees a clean connection
    // close rather than RST (RFC 9113 §6.8).
    let _ = io.shutdown(Shutdown::Write).await;

    res
}

enum ShutDown {
    ReadClosed(io::Result<()>),
    WriteClosed(io::Result<()>),
    Timeout(io::Error),
}

// only check the prefix but not consume it
async fn prefix_check<const LIMIT: usize>(
    mut read_buf: BytesMut,
    io: &(impl AsyncBufRead + AsyncBufWrite),
) -> (BytesMut, io::Result<()>) {
    let mut res = Ok(());

    while read_buf.len() < PREFACE.len() {
        let (read_res, b) = read_io::<LIMIT>(read_buf, io).await;
        read_buf = b;

        let e = match read_res {
            Ok(0) => io::ErrorKind::UnexpectedEof.into(),
            Ok(_) => {
                let n = read_buf.len().min(PREFACE.len());
                if read_buf[..n] == PREFACE[..n] {
                    continue;
                }

                io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP/2 client preface")
            }
            Err(e) => e,
        };

        res = Err(e);
        break;
    }

    (read_buf, res)
}

#[cold]
#[inline(never)]
async fn lingering_read(io: &impl AsyncBufRead, mut ka: Pin<&mut KeepAlive>, date: &DateTimeHandle) -> io::Result<()> {
    ka.as_mut().update(date.now() + Duration::from_secs(5));
    ka.as_mut().reset();

    let mut read_buf = BytesMut::with_capacity(4096);

    loop {
        read_buf.clear();

        match io.read(read_buf).timeout(ka.as_mut()).await {
            Ok((res, buf)) => {
                read_buf = buf;

                if res? == 0 {
                    return Ok(());
                }
            }
            Err(_) => return Ok(()),
        }
    }
}

type BoxedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

// Perform the HTTP/2 connection handshake: validate the client preface,
// then send our SETTINGS frame. Returns the read buffer (with preface
// consumed) and the write buffer (ready for reuse).
#[cold]
#[inline(never)]
fn handshake<'a, const LIMIT: usize>(
    io: &'a (impl AsyncBufRead + AsyncBufWrite),
    buf: BytesMut,
    timer: Pin<&'a mut KeepAlive>,
) -> BoxedFuture<'a, io::Result<BytesMut>> {
    Box::pin(async move {
        async {
            // No cap during preface: the buffer is tiny and always fully drained.
            let (mut read_buf, res) = prefix_check::<LIMIT>(buf, io).await;
            res.map(|_| {
                read_buf.advance(PREFACE.len());
                read_buf
            })
        }
        .timeout(timer)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "h2 handshake timeout"))?
    })
}
