#![allow(dead_code)]

mod data;
mod dispatcher;
mod error;
mod go_away;
mod head;
mod headers;
mod hpack;
mod priority;
mod reason;
mod reset;
mod settings;
mod stream_id;
mod window_update;

pub(crate) use dispatcher::Dispatcher;

const HEADER_LEN: usize = 9;

#[cfg(feature = "io-uring")]
pub use io_uring::{run, RequestBodySender, RequestBodyV2};

#[cfg(feature = "io-uring")]
mod io_uring {
    use core::{
        cell::RefCell,
        fmt,
        future::{poll_fn, Future},
        mem,
        pin::{pin, Pin},
        task::{Context, Poll, Waker},
    };

    use std::{collections::HashMap, io};

    use futures_core::stream::Stream;
    use pin_project_lite::pin_project;
    use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
    use tracing::error;
    use xitca_io::{
        bytes::{Buf, BufMut, BytesMut},
        io_uring::{write_all, AsyncBufRead, AsyncBufWrite, IoBuf},
    };
    use xitca_service::Service;
    use xitca_unsafe_collection::futures::{Select, SelectOutput};

    use crate::{
        body::BodySize,
        bytes::Bytes,
        error::BodyError,
        http::{header::CONTENT_LENGTH, HeaderMap, Request, RequestExt, Response, Version},
        util::futures::Queue,
    };

    use super::{
        data,
        error::Error,
        go_away::GoAway,
        head, headers, hpack,
        reason::Reason,
        reset::Reset,
        settings::{self, Settings},
        stream_id::StreamId,
        window_update::WindowUpdate,
    };

    const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

    struct DecodeContext<'a> {
        max_header_list_size: usize,
        remote_setting: Settings,
        decoder: hpack::Decoder,
        // next_frame_len == 0 is used as maker for waiting for new frame.
        next_frame_len: usize,
        continuation: Option<(headers::Headers, BytesMut)>,
        shared_state: &'a RefCell<SharedState>,
        stream_map: &'a StreamMap,
        stream_map2: HashMap<StreamId, RequestBodySender>,
        writer_tx: &'a UnboundedSender<Message>,
    }

    enum Message {
        Head(StreamId, Response<()>),
        Stream(StreamId, Bytes),
        Trailer(StreamId, HeaderMap),
        Reset(StreamId, Reason),
        WindowUpdate(StreamId, usize),
        Settings,
    }

    struct SharedState {
        conn_window: usize,
        stream_window: usize,
    }

    type StreamMap = RefCell<HashMap<StreamId, StreamControlFlow>>;

    impl<'a> DecodeContext<'a> {
        fn new(
            shared_state: &'a RefCell<SharedState>,
            stream_map: &'a StreamMap,
            writer_tx: &'a UnboundedSender<Message>,
        ) -> Self {
            Self {
                remote_setting: Settings::default(),
                max_header_list_size: settings::DEFAULT_SETTINGS_MAX_HEADER_LIST_SIZE,
                decoder: hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE),
                next_frame_len: 0,
                continuation: None,
                shared_state,
                stream_map,
                stream_map2: HashMap::new(),
                writer_tx,
            }
        }

        async fn try_decode<F>(&mut self, buf: &mut BytesMut, mut on_msg: F) -> Result<(), Error>
        where
            F: FnMut(Request<RequestExt<RequestBodyV2>>, StreamId),
        {
            loop {
                if self.next_frame_len == 0 {
                    if buf.len() < 3 {
                        return Ok(());
                    }
                    self.next_frame_len = (buf.get_uint(3) + 6) as _;
                }

                if buf.len() < self.next_frame_len {
                    return Ok(());
                }

                let len = mem::replace(&mut self.next_frame_len, 0);
                let mut frame = buf.split_to(len);
                let head = head::Head::parse(&frame);

                // TODO: Make Head::parse auto advance the frame?
                frame.advance(6);

                match head.kind() {
                    head::Kind::Settings => {
                        let setting = Settings::load(head, &frame)?;
                        if !setting.is_ack() {
                            self.remote_setting = setting;
                            self.writer_tx.send(Message::Settings).unwrap();
                        }
                    }
                    head::Kind::Headers => {
                        let (mut headers, mut payload) = headers::Headers::load(head, frame)?;

                        let is_end_headers = headers.is_end_headers();

                        headers.load_hpack(&mut payload, self.max_header_list_size, &mut self.decoder)?;

                        if !is_end_headers {
                            self.continuation = Some((headers, payload));
                            continue;
                        }

                        let id = headers.stream_id();

                        self.handle_header_frame(id, headers, &mut on_msg);
                    }
                    head::Kind::Continuation => {
                        let is_end_headers = (head.flag() & 0x4) == 0x4;

                        let Some((mut headers, mut payload)) = self.continuation.take() else {
                            panic!("illegal continuation frame");
                        };

                        let id = headers.stream_id();

                        if id != head.stream_id() {
                            panic!("CONTINUATION frame stream ID does not match previous frame stream ID");
                        }

                        payload.extend_from_slice(&frame);

                        if let Err(e) = headers.load_hpack(&mut payload, self.max_header_list_size, &mut self.decoder) {
                            match e {
                                Error::Hpack(hpack::DecoderError::NeedMore(_)) if !is_end_headers => {
                                    self.continuation = Some((headers, payload));
                                    continue;
                                }
                                e => return Err(e),
                            }
                        }

                        self.handle_header_frame(id, headers, &mut on_msg);
                    }
                    head::Kind::Data => {
                        let data = data::Data::load(head, frame.freeze())?;
                        let mut is_end = data.is_end_stream();
                        let id = data.stream_id();
                        let payload = data.into_payload();

                        if !payload.is_empty() {
                            let tx = self.stream_map2.get(&id).unwrap();

                            if tx.send(Ok(payload)).is_err() {
                                self.writer_tx.send(Message::Reset(id, Reason::CANCEL)).unwrap();
                                is_end = true;
                            };
                        }

                        if is_end {
                            self.stream_map2.remove(&id);
                        }
                    }
                    head::Kind::WindowUpdate => {
                        let window = WindowUpdate::load(head, frame.as_ref())?;

                        if window.stream_id() == 0 {
                            // connection window
                        } else {
                            let mut map = self.stream_map.borrow_mut();
                            let flow = &mut map.get_mut(&window.stream_id()).unwrap();
                            flow.window += window.size_increment() as usize;
                            if let Some(waker) = flow.waker.take() {
                                waker.wake();
                            }
                        }
                    }
                    head::Kind::GoAway => {
                        let go_away = GoAway::load(frame.as_ref())?;
                        assert_eq!(go_away.reason(), Reason::NO_ERROR);
                    }
                    _ => {}
                }
            }
        }

        fn handle_header_frame<F>(&mut self, id: StreamId, headers: headers::Headers, on_msg: &mut F)
        where
            F: FnMut(Request<RequestExt<RequestBodyV2>>, StreamId),
        {
            let is_end_stream = headers.is_end_stream();

            let (pseudo, headers) = headers.into_parts();

            let req = match self.stream_map2.remove(&id) {
                Some(_) => {
                    error!("trailer is not supported yet");
                    return;
                }
                None => {
                    let mut req = Request::new(RequestExt::<()>::default());
                    *req.version_mut() = Version::HTTP_2;
                    *req.headers_mut() = headers;
                    *req.method_mut() = pseudo.method.unwrap();
                    req
                }
            };

            let (body, tx) = RequestBodyV2::new_pair(id, self.writer_tx.clone());

            if is_end_stream {
                drop(tx);
            } else {
                self.stream_map2.insert(id, tx);
                self.stream_map.borrow_mut().insert(
                    id,
                    StreamControlFlow {
                        window: 65535,
                        frame_size: self.remote_setting.max_frame_size().unwrap_or(16384) as usize,
                        waker: None,
                    },
                );
            };

            let req = req.map(|ext| ext.map_body(|_| body));

            on_msg(req, id);
        }
    }

    async fn read_io(mut buf: BytesMut, io: &impl AsyncBufRead) -> (io::Result<usize>, BytesMut) {
        let len = buf.len();
        let remaining = buf.capacity() - len;
        if remaining < 4096 {
            buf.reserve(4096 - remaining);
        }
        let (res, buf) = io.read(buf.slice(len..)).await;
        (res, buf.into_inner())
    }

    async fn write_io(buf: BytesMut, io: &impl AsyncBufWrite) -> (io::Result<()>, BytesMut) {
        let (res, mut buf) = write_all(io, buf).await;
        buf.clear();
        (res, buf)
    }

    pin_project! {
        #[project = CompleteTaskProj]
        #[project_replace = CompleteTaskReplaceProj]
        enum CompleteTask<F> {
            Task {
                #[pin]
                fut: F
            },
            Idle
        }
    }

    struct StreamControlFlow {
        window: usize,
        frame_size: usize,
        waker: Option<Waker>,
    }

    impl<F> Future for CompleteTask<F>
    where
        F: Future,
    {
        type Output = F::Output;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            match self.project() {
                CompleteTaskProj::Task { fut } => fut.poll(cx),
                CompleteTaskProj::Idle => Poll::Pending,
            }
        }
    }

    /// Experimental h2 http layer.
    pub async fn run<Io, S, ResB, ResBE>(io: Io, service: S) -> io::Result<()>
    where
        Io: AsyncBufRead + AsyncBufWrite,
        S: Service<Request<RequestExt<RequestBodyV2>>, Response = Response<ResB>>,
        S::Error: fmt::Debug,
        ResB: Stream<Item = Result<Bytes, ResBE>>,
        ResBE: fmt::Debug,
    {
        let mut read_buf = BytesMut::new();
        let mut write_buf = BytesMut::new();

        read_buf = prefix_check(&io, read_buf).await?;

        let mut settings = settings::Settings::default();
        settings.set_max_concurrent_streams(Some(256));

        settings.encode(&mut write_buf);
        let (res, buf) = write_io(write_buf, &io).await;
        write_buf = buf;
        res?;

        let mut read_task = pin!(read_io(read_buf, &io));

        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

        let shared_state = RefCell::new(SharedState {
            conn_window: settings.initial_window_size().unwrap_or(65535) as _,
            stream_window: usize::MIN,
        });

        let stream_map = RefCell::new(HashMap::new());

        let mut ctx = DecodeContext::new(&shared_state, &stream_map, &tx);
        let mut queue = Queue::new();

        let mut write_task = pin!(async {
            let mut encoder = hpack::Encoder::new(65535, 4096);

            use core::ops::ControlFlow;

            enum State {
                Write,
                WriteEof,
            }

            loop {
                let state = poll_fn(|cx| match rx.poll_recv(cx) {
                    Poll::Ready(Some(msg)) => {
                        match msg {
                            Message::Head(id, res) => {
                                let (parts, _) = res.into_parts();
                                let pseudo = headers::Pseudo::response(parts.status);
                                let headers = headers::Headers::new(id, pseudo, parts.headers);
                                let mut buf = (&mut write_buf).limit(4096);
                                headers.encode(&mut encoder, &mut buf);
                            }
                            Message::Stream(id, bytes) => {
                                let mut data = data::Data::new(id, bytes);
                                data.encode_chunk(&mut write_buf);
                            }
                            Message::Trailer(stream_id, map) => {
                                let trailer = headers::Headers::trailers(stream_id, map);
                                let mut buf = (&mut write_buf).limit(4096);
                                trailer.encode(&mut encoder, &mut buf);
                            }
                            Message::Reset(id, reason) => {
                                let reset = Reset::new(id, reason);
                                reset.encode(&mut write_buf);
                            }
                            Message::WindowUpdate(id, size) => {
                                debug_assert!(size > 0, "window update size not be 0");
                                // TODO: batch window update
                                let update = WindowUpdate::new(id, size as _);
                                update.encode(&mut write_buf);
                                let update = WindowUpdate::new(0.into(), size as _);
                                update.encode(&mut write_buf);
                                shared_state.borrow_mut().conn_window += size;
                            }
                            Message::Settings => {
                                let setting = Settings::ack();
                                setting.encode(&mut write_buf);
                            }
                        };
                        Poll::Ready(ControlFlow::Continue(()))
                    }
                    Poll::Pending if write_buf.is_empty() => Poll::Pending,
                    Poll::Pending => Poll::Ready(ControlFlow::Break(State::Write)),
                    Poll::Ready(None) => Poll::Ready(ControlFlow::Break(State::WriteEof)),
                })
                .await;

                match state {
                    ControlFlow::Continue(_) => continue,
                    ControlFlow::Break(state) => {
                        let (res, buf) = write_io(write_buf, &io).await;
                        write_buf = buf;

                        res?;

                        if matches!(state, State::WriteEof) {
                            return Ok::<_, io::Error>(());
                        }
                    }
                }
            }
        });

        loop {
            match read_task
                .as_mut()
                .select(queue.next())
                .select(write_task.as_mut())
                .await
            {
                SelectOutput::A(SelectOutput::A((res, buf))) => {
                    read_buf = buf;
                    if res? == 0 {
                        break;
                    }

                    let res = ctx
                        .try_decode(&mut read_buf, |req, stream_id| {
                            let s = &service;
                            let t = &tx;
                            let map = &stream_map;

                            queue.push(async move {
                                match s.call(req).await {
                                    Ok(res) => {
                                        let (mut parts, body) = res.into_parts();

                                        if let BodySize::Sized(size) = BodySize::from_stream(&body) {
                                            parts.headers.insert(CONTENT_LENGTH, size.into());
                                        }

                                        t.send(Message::Head(stream_id, Response::from_parts(parts, ())))
                                            .unwrap();

                                        let mut body = pin!(body);

                                        while let Some(chunk) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                                            let mut bytes = chunk.unwrap();

                                            while !bytes.is_empty() {
                                                let len = bytes.len();

                                                let aval = poll_fn(|cx| {
                                                    let mut map = map.borrow_mut();
                                                    let flow = map.get_mut(&stream_id).unwrap();

                                                    if flow.window == 0 {
                                                        flow.waker = Some(cx.waker().clone());
                                                        return Poll::Pending;
                                                    }

                                                    let len = core::cmp::min(len, flow.frame_size);
                                                    let aval = core::cmp::min(flow.window, len);
                                                    flow.window -= aval;
                                                    Poll::Ready(aval)
                                                })
                                                .await;

                                                let chunk = bytes.split_to(aval);
                                                t.send(Message::Stream(stream_id, chunk)).unwrap();
                                            }
                                        }

                                        t.send(Message::Trailer(stream_id, HeaderMap::new())).unwrap();
                                    }
                                    Err(e) => error!("service error: {:?}", e),
                                };
                            });
                        })
                        .await;

                    if let Err(e) = res {
                        panic!("decode error: {e:?}")
                    }

                    read_task.set(read_io(read_buf, &io));
                }
                SelectOutput::A(SelectOutput::B(_)) => {}
                SelectOutput::B(_) => break,
            }
        }

        drop(queue);
        drop(tx);

        Ok(())
    }

    #[cold]
    #[inline(never)]
    async fn prefix_check(io: &impl AsyncBufRead, mut buf: BytesMut) -> io::Result<BytesMut> {
        while buf.len() < PREFACE.len() {
            let (res, b) = read_io(buf, io).await;
            buf = b;
            res?;
        }

        if &buf[..PREFACE.len()] == PREFACE {
            buf.advance(PREFACE.len());
        } else {
            todo!()
        }

        Ok(buf)
    }

    /// Request body type for Http/2 specifically.
    pub struct RequestBodyV2 {
        stream_id: StreamId,
        rx: UnboundedReceiver<Result<Bytes, BodyError>>,
        writer_tx: UnboundedSender<Message>,
    }

    pub type RequestBodySender = UnboundedSender<Result<Bytes, BodyError>>;

    impl RequestBodyV2 {
        #[cfg(feature = "io-uring")]
        fn new_pair(stream_id: StreamId, writer_tx: UnboundedSender<Message>) -> (Self, RequestBodySender) {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            (
                Self {
                    stream_id,
                    rx,
                    writer_tx,
                },
                tx,
            )
        }
    }

    impl Stream for RequestBodyV2 {
        type Item = Result<Bytes, BodyError>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.get_mut();

            this.rx.poll_recv(cx).map(|opt| {
                opt.map(|res| {
                    let bytes = res?;
                    let _ = this.writer_tx.send(Message::WindowUpdate(this.stream_id, bytes.len()));
                    Ok(bytes)
                })
            })
        }
    }
}

/// A helper macro that unpacks a sequence of 4 bytes found in the buffer with
/// the given identifier, starting at the given offset, into the given integer
/// type. Obviously, the integer type should be able to support at least 4
/// bytes.
///
/// # Examples
///
/// ```ignore
/// # // We ignore this doctest because the macro is not exported.
/// let buf: [u8; 4] = [0, 0, 0, 1];
/// assert_eq!(1u32, unpack_octets_4!(buf, 0, u32));
/// ```
macro_rules! unpack_octets_4 {
    // TODO: Get rid of this macro
    ($buf:expr, $offset:expr, $tip:ty) => {
        (($buf[$offset + 0] as $tip) << 24)
            | (($buf[$offset + 1] as $tip) << 16)
            | (($buf[$offset + 2] as $tip) << 8)
            | (($buf[$offset + 3] as $tip) << 0)
    };
}

use unpack_octets_4;

#[cfg(test)]
mod tests {
    #[test]
    fn test_unpack_octets_4() {
        let buf: [u8; 4] = [0, 0, 0, 1];
        assert_eq!(1u32, unpack_octets_4!(buf, 0, u32));
    }
}
