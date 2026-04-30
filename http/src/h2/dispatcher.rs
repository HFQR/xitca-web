use core::{
    cell::{RefCell, RefMut},
    fmt,
    future::poll_fn,
    mem,
    net::SocketAddr,
    pin::{Pin, pin},
    task::{Context, Poll},
    time::Duration,
};

use std::{io, net::Shutdown, rc::Rc};

use tracing::error;
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncBufRead, AsyncBufWrite, BoundedBuf, write_all},
};
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::{
    body::{Body, SizeHint},
    bytes::Bytes,
    config::HttpServiceConfig,
    date::{DateTime, DateTimeHandle},
    http::{
        Extension, Method, Request, RequestExt, Response, Version,
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
        error::Error as ProtoError,
        flow::{DecodedRequest, FlowControlClone, FlowControlLock},
        flow::{Error as FlowError, FlowControl, Frame},
        frame::{
            PREFACE,
            data::Data,
            go_away::GoAway,
            head, headers,
            ping::Ping,
            reason::Reason,
            reset::Reset,
            settings::{self},
            stream_id::StreamId,
            window_update::WindowUpdate,
        },
        hpack,
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

struct DecodeContext {
    max_frame_size: usize,
    max_header_list_size: usize,
    decoder: hpack::Decoder,
    next_frame_len: usize,
    continuation: Option<(headers::Headers, BytesMut)>,
}

impl DecodeContext {
    fn try_decode(&mut self, buf: &mut BytesMut, flow: &mut FlowControl) -> Result<Option<DecodedRequest>, FlowError> {
        loop {
            if self.next_frame_len == 0 {
                if buf.len() < 3 {
                    return Ok(None);
                }
                let payload_len = buf.get_uint(3) as usize;
                if payload_len > self.max_frame_size {
                    return Err(FlowError::GoAway(Reason::FRAME_SIZE_ERROR));
                }
                self.next_frame_len = payload_len + 6;
            }

            if buf.len() < self.next_frame_len {
                return Ok(None);
            }

            let len = mem::replace(&mut self.next_frame_len, 0);
            let mut frame = buf.split_to(len);
            let head = head::Head::parse(&frame);

            // TODO: Make Head::parse auto advance the frame?
            frame.advance(6);

            if let Some(decoded) = self.decode_frame(head, frame, flow)? {
                return Ok(Some(decoded));
            }
        }
    }

    fn decode_frame(
        &mut self,
        head: head::Head,
        frame: BytesMut,
        flow: &mut FlowControl,
    ) -> Result<Option<DecodedRequest>, FlowError> {
        match self._decode_frame(head, frame, flow) {
            Err(FlowError::Reset(reason)) => {
                flow.try_push_reset(head.stream_id(), reason)?;
                Ok(None)
            }
            res => res,
        }
    }

    fn _decode_frame(
        &mut self,
        head: head::Head,
        frame: BytesMut,
        flow: &mut FlowControl,
    ) -> Result<Option<DecodedRequest>, FlowError> {
        if self.continuation.is_some() && !matches!(head.kind(), head::Kind::Continuation) {
            return Err(FlowError::GoAway(Reason::PROTOCOL_ERROR));
        }

        match head.kind() {
            head::Kind::Headers => {
                let (headers, payload) = headers::Headers::load(head, frame)?;
                let is_end_headers = headers.is_end_headers();
                return self.handle_header(headers, payload, is_end_headers, flow);
            }
            head::Kind::Data => {
                let data = Data::load(head, frame.freeze())?;
                flow.recv_data(data)?;
            }
            head::Kind::WindowUpdate => {
                let window = WindowUpdate::load(head, frame.as_ref())?;
                flow.recv_window_update(window)?;
            }
            head::Kind::Ping => {
                let ping = Ping::load(head, frame.as_ref())?;
                flow.recv_ping(ping);
            }
            head::Kind::Reset => {
                let reset = Reset::load(head, frame.as_ref())?;
                flow.recv_reset(reset)?;
            }
            head::Kind::GoAway => {
                let go_away = GoAway::load(head.stream_id(), frame.as_ref())?;
                return Err(FlowError::GoAway(go_away.reason()));
            }
            head::Kind::Continuation => return self.handle_continuation(head, frame, flow),
            head::Kind::Priority => handle_priority(head.stream_id(), &frame)?,
            head::Kind::PushPromise => return Err(FlowError::GoAway(Reason::PROTOCOL_ERROR)),
            head::Kind::Settings => flow.recv_setting(head, &frame)?,
            head::Kind::Unknown => {}
        }
        Ok(None)
    }

    #[cold]
    #[inline(never)]
    fn handle_continuation(
        &mut self,
        head: head::Head,
        frame: BytesMut,
        flow: &mut FlowControl,
    ) -> Result<Option<DecodedRequest>, FlowError> {
        let is_end_headers = (head.flag() & 0x4) == 0x4;

        let (headers, mut payload) = self
            .continuation
            .take()
            .ok_or(FlowError::GoAway(Reason::PROTOCOL_ERROR))?;

        // RFC 7540 §6.10: CONTINUATION without a preceding incomplete HEADERS
        // is a connection error PROTOCOL_ERROR
        if headers.stream_id() != head.stream_id() {
            return Err(FlowError::GoAway(Reason::PROTOCOL_ERROR));
        }

        payload.unsplit(frame);
        self.handle_header(headers, payload, is_end_headers, flow)
    }

    fn handle_header(
        &mut self,
        mut headers: headers::Headers,
        mut payload: BytesMut,
        is_end_headers: bool,
        flow: &mut FlowControl,
    ) -> Result<Option<DecodedRequest>, FlowError> {
        if let Err(e) = headers.load_hpack(&mut payload, self.max_header_list_size, &mut self.decoder) {
            return match e {
                // NeedMore on a multi-frame header block is normal; accumulate and wait
                // for CONTINUATION frames (RFC 7540 §6.10).
                ProtoError::Hpack(hpack::DecoderError::NeedMore(_)) if !is_end_headers => {
                    self.continuation = Some((headers, payload));
                    Ok(None)
                }
                // Pseudo-header validation errors are stream-level (RFC 7540 §8.1.2).
                // The HPACK context was decoded successfully so no compression error.
                ProtoError::MalformedMessage => {
                    let id = headers.stream_id();
                    if flow.try_set_last_stream_id(id)?.is_none() {
                        return Ok(None);
                    }
                    Err(FlowError::Reset(Reason::PROTOCOL_ERROR))
                }
                _ => Err(FlowError::GoAway(Reason::COMPRESSION_ERROR)),
            };
        }

        if !is_end_headers {
            self.continuation = Some((headers, payload));
            return Ok(None);
        }

        let id = headers.stream_id();

        flow.recv_header(id, headers)
    }
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
            ctx: DecodeContext {
                max_frame_size,
                max_header_list_size,
                decoder: hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE),
                next_frame_len: 0,
                continuation: None,
            },
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

async fn write_io(buf: BytesMut, io: &impl AsyncBufWrite) -> (io::Result<()>, BytesMut) {
    let (res, mut buf) = write_all(io, buf).await;
    buf.clear();
    (res, buf)
}

struct Encoder<'a> {
    encoder: hpack::Encoder,
    flow: &'a FlowControlLock,
}

impl<'a> Encoder<'a> {
    fn new(flow: &'a FlowControlLock) -> Self {
        Self {
            encoder: hpack::Encoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE, 4096),
            flow,
        }
    }

    fn poll_encode(&mut self, write_buf: &mut BytesMut, cx: &mut Context<'_>) -> Poll<bool> {
        self.flow.borrow_mut().poll_encode(write_buf, &mut self.encoder, cx)
    }
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
    _response_task(req, stream_id, service, ctx, date)
        .await
        .unwrap_or_else(|_| {
            let mut flow = ctx.borrow_mut();
            flow.internal_reset(&stream_id);
            flow
        })
        .response_task_done(stream_id)
}

// clippy is dumb
#[allow(clippy::await_holding_refcell_ref)]
async fn _response_task<'a, S, ReqB, ResB, ResBE>(
    req: Request<RequestExt<RequestBody>>,
    stream_id: StreamId,
    service: &S,
    flow: &'a FlowControlLock,
    date: &DateTimeHandle,
) -> Result<RefMut<'a, FlowControl>, ()>
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

    if !parts.headers.contains_key(DATE) {
        let date = date.with_date_header(Clone::clone);
        parts.headers.insert(DATE, date);
    }

    let end_stream = match (head_method, body.size_hint()) {
        (true, _) => true,
        (false, SizeHint::None) => true,
        (false, size) => {
            if let SizeHint::Exact(size) = size {
                parts.headers.entry(CONTENT_LENGTH).or_insert_with(|| size.into());
            }
            false
        }
    };

    let pseudo = headers::Pseudo::response(parts.status);
    let mut headers = headers::Headers::new(stream_id, pseudo, parts.headers);

    if end_stream {
        headers.set_end_stream();
    }

    let mut f = flow.borrow_mut();

    f.send_headers(headers);

    if !end_stream {
        drop(f);

        let mut body = pin!(body);

        f = loop {
            match poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
                None => {
                    let mut f = flow.borrow_mut();
                    f.send_end_stream(stream_id);
                    break f;
                }
                Some(Err(e)) => {
                    error!("body error: {e:?}");
                    return Err(());
                }
                Some(Ok(Frame::Data(mut data))) => {
                    if data.is_empty() && !end_stream {
                        tracing::warn!(
                            "response body should not yield empty Frame::Data unless it's the last chunk of Body"
                        );
                        continue;
                    }

                    let opt = poll_fn(|cx| {
                        let mut flow = flow.borrow_mut();
                        flow.poll_send_data(stream_id, &mut data, end_stream, cx)
                            .map(|opt| opt.map(|_| flow))
                    })
                    .await;

                    if let Some(flow) = opt {
                        break flow;
                    }
                }
                Some(Ok(Frame::Trailers(trailers))) => {
                    let mut f = flow.borrow_mut();
                    f.send_trailers(stream_id, trailers);
                    break f;
                }
            }
        }
    }

    Ok(f)
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

    let mut flow = FlowControl::new(&settings);

    let (mut read_buf, mut write_buf) = flow
        .handshake::<READ_BUF_LIMIT>(&io, read_buf, &settings, ka.as_mut())
        .await?;

    let flow = Rc::new(RefCell::new(flow));

    let mut ctx = Decoder::new(&flow, service, max_frame_size, max_header_list_size, addr, date);
    let mut enc = Encoder::new(&flow);

    let mut queue = Queue::new();
    let mut ping_pong = PingPong::new(ka.as_mut(), &flow, date, config.keep_alive_timeout);

    let res = {
        let mut read_task = pin!(read_io::<READ_BUF_LIMIT>(read_buf, &io));

        let mut write_task = pin!(async {
            while poll_fn(|cx| enc.poll_encode(&mut write_buf, cx)).await {
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
    // close rather than RST (RFC 7540 §6.8).
    let _ = io.shutdown(Shutdown::Write).await;

    res
}

enum ShutDown {
    ReadClosed(io::Result<()>),
    WriteClosed(io::Result<()>),
    Timeout(io::Error),
}

/// Validate a PRIORITY frame payload (RFC 7540 §6.3, §5.3.1).
/// PRIORITY frames are deprecated in RFC 9113 and their content is ignored,
/// but the structural rules must still be enforced for RFC 7540 compatibility.
#[cold]
#[inline(never)]
fn handle_priority(id: StreamId, payload: &[u8]) -> Result<(), FlowError> {
    if id.is_zero() {
        Err(FlowError::GoAway(Reason::PROTOCOL_ERROR))
    } else if payload.len() != 5 {
        Err(FlowError::Reset(Reason::FRAME_SIZE_ERROR))
    } else if id == StreamId::parse(&payload[..4]).0 {
        Err(FlowError::Reset(Reason::PROTOCOL_ERROR))
    } else {
        Ok(())
    }
}

// only check the prefix but not consume it
async fn prefix_check<const LIMIT: usize>(
    mut read_buf: BytesMut,
    io: &(impl AsyncBufRead + AsyncBufWrite),
) -> (BytesMut, io::Result<()>) {
    while read_buf.len() < PREFACE.len() {
        let (res, b) = read_io::<LIMIT>(read_buf, io).await;
        read_buf = b;

        let err = match res {
            Ok(0) => io::ErrorKind::UnexpectedEof.into(),
            Ok(_) => continue,
            Err(e) => e,
        };

        return (read_buf, Err(err));
    }

    let res = if !read_buf.starts_with(PREFACE) {
        // No GOAWAY is sent because our own preface has not been exchanged yet.
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP/2 client preface",
        ))
    } else {
        Ok(())
    };

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

impl FlowControl {
    /// Perform the HTTP/2 connection handshake: validate the client preface,
    /// then send our SETTINGS frame. Returns the read buffer (with preface
    /// consumed) and the write buffer (ready for reuse).
    #[cold]
    #[inline(never)]
    pub(crate) fn handshake<'a, const LIMIT: usize>(
        &'a mut self,
        io: &'a (impl AsyncBufRead + AsyncBufWrite),
        buf: BytesMut,
        settings: &'a settings::Settings,
        timer: Pin<&'a mut KeepAlive>,
    ) -> BoxedFuture<'a, io::Result<(BytesMut, BytesMut)>> {
        Box::pin(async move {
            async {
                // No cap during preface: the buffer is tiny and always fully drained.
                let (mut read_buf, res) = prefix_check::<LIMIT>(buf, io).await;
                res?;
                read_buf.advance(PREFACE.len());

                let mut write_buf = BytesMut::new();
                settings.encode(&mut write_buf);

                self.encode_init_window_update(&mut write_buf);

                let (res, write_buf) = write_io(write_buf, io).await;
                res?;

                Ok((read_buf, write_buf))
            }
            .timeout(timer)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "h2 handshake timeout"))?
        })
    }
}
