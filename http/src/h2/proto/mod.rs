#![allow(dead_code)]

mod data;
mod dispatcher;
mod head;
mod headers;
mod hpack;
mod priority;
mod settings;
mod stream_id;

pub(crate) use dispatcher::Dispatcher;

const HEADER_LEN: usize = 9;

#[cfg(feature = "io-uring")]
pub use io_uring::run;

#[cfg(feature = "io-uring")]
mod io_uring {
    use core::{
        convert::Infallible,
        future::Future,
        mem,
        pin::{pin, Pin},
        task::{Context, Poll},
    };

    use std::{collections::HashMap, io};

    use pin_project_lite::pin_project;
    use tracing::error;
    use xitca_io::{
        bytes::{Buf, BufMut, BytesMut},
        io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf},
    };
    use xitca_service::Service;
    use xitca_unsafe_collection::futures::{Select, SelectOutput};

    use crate::{
        h2::{RequestBodySender, RequestBodyV2},
        http::{Request, RequestExt, Response, Version},
        util::futures::Queue,
    };

    use super::{data, head, headers, hpack, settings, stream_id};

    const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

    struct H2Context {
        decoder: hpack::Decoder,
        encoder: hpack::Encoder,
        tx_map: HashMap<stream_id::StreamId, RequestBodySender>,
        // next_frame_len == 0 is used as maker for waiting for new frame.
        next_frame_len: usize,
        continuation: Option<(headers::Headers, BytesMut)>,
    }

    impl Default for H2Context {
        fn default() -> Self {
            Self::new()
        }
    }

    impl H2Context {
        fn new() -> Self {
            Self {
                decoder: hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE),
                encoder: hpack::Encoder::new(65535, 4096),
                tx_map: HashMap::new(),
                next_frame_len: 0,
                continuation: None,
            }
        }

        fn try_decode<F>(&mut self, buf: &mut BytesMut, mut on_msg: F)
        where
            F: FnMut(Request<RequestExt<RequestBodyV2>>, stream_id::StreamId),
        {
            loop {
                if self.next_frame_len == 0 {
                    if buf.len() < 3 {
                        return;
                    }
                    self.next_frame_len = (buf.get_uint(3) + 6) as _;
                }

                if buf.len() < self.next_frame_len {
                    return;
                }

                let len = mem::replace(&mut self.next_frame_len, 0);
                let mut frame = buf.split_to(len);
                let head = head::Head::parse(&frame);

                // TODO: Make Head::parse auto advance the frame?
                frame.advance(6);

                match head.kind() {
                    head::Kind::Settings => {
                        let _setting = settings::Settings::load(head, &frame);
                    }
                    head::Kind::Headers => {
                        let (mut headers, mut payload) = headers::Headers::load(head, frame).unwrap();

                        let is_end_headers = headers.is_end_headers();

                        headers.load_hpack(&mut payload, 4096, &mut self.decoder).unwrap();

                        if !is_end_headers {
                            self.continuation = Some((headers, payload));
                            continue;
                        }

                        let id = headers.stream_id();
                        let is_end_stream = headers.is_end_stream();

                        let (pseudo, headers) = headers.into_parts();

                        let req = match self.tx_map.remove(&id) {
                            Some(_) => {
                                error!("trailer is not supported yet");
                                continue;
                            }
                            None => {
                                let mut req = Request::new(RequestExt::<()>::default());
                                *req.version_mut() = Version::HTTP_2;
                                *req.headers_mut() = headers;
                                *req.method_mut() = pseudo.method.unwrap();
                                req
                            }
                        };

                        let (body, tx) = RequestBodyV2::new_pair();

                        if is_end_stream {
                            drop(tx);
                        } else {
                            self.tx_map.insert(id, tx);
                        }

                        let req = req.map(|ext| ext.map_body(|_| body));

                        on_msg(req, id);
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

                        headers.load_hpack(&mut payload, 4096, &mut self.decoder).unwrap();

                        if !is_end_headers {
                            self.continuation = Some((headers, payload));
                            continue;
                        }

                        let is_end_stream = headers.is_end_stream();

                        let (pseudo, headers) = headers.into_parts();

                        let req = match self.tx_map.remove(&id) {
                            Some(_) => {
                                error!("trailer is not supported yet");
                                continue;
                            }
                            None => {
                                let mut req = Request::new(RequestExt::<()>::default());
                                *req.version_mut() = Version::HTTP_2;
                                *req.headers_mut() = headers;
                                *req.method_mut() = pseudo.method.unwrap();
                                req
                            }
                        };

                        let (body, tx) = RequestBodyV2::new_pair();

                        if is_end_stream {
                            drop(tx);
                        } else {
                            self.tx_map.insert(id, tx);
                        };

                        let req = req.map(|ext| ext.map_body(|_| body));

                        on_msg(req, id);
                    }
                    head::Kind::Data => {
                        let data = data::Data::load(head, frame.freeze()).unwrap();
                        let is_end = data.is_end_stream();
                        let id = data.stream_id();
                        let payload = data.into_payload();

                        let tx = self.tx_map.get_mut(&id).unwrap();

                        tx.send(Ok(payload)).unwrap();

                        if is_end {
                            self.tx_map.remove(&id);
                        }
                    }
                    _ => {}
                }
            }
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
        async fn write_all(io: &impl AsyncBufWrite, mut buf: BytesMut) -> (io::Result<()>, BytesMut) {
            let mut n = 0;
            while n < buf.bytes_init() {
                match io.write(buf.slice(n..)).await {
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
    pub async fn run<Io, S>(io: Io, service: S) -> io::Result<()>
    where
        Io: AsyncBufRead + AsyncBufWrite,
        S: Service<Request<RequestExt<RequestBodyV2>>, Response = Response<()>, Error = Infallible>,
    {
        let mut read_buf = Some(BytesMut::new());
        let mut write_buf = Some(BytesMut::new());

        prefix_check(&io, &mut read_buf).await?;

        let settings = settings::Settings::default();

        settings.encode(write_buf.as_mut().unwrap());
        let (res, buf) = write_io(write_buf.take().unwrap(), &io).await;
        write_buf = Some(buf);
        res?;

        let mut ctx = H2Context::new();
        let mut queue = Queue::new();

        let mut read_task = pin!(read_io(read_buf.take().unwrap(), &io));

        loop {
            match read_task.as_mut().select(queue.next()).await {
                SelectOutput::A((res, buf)) => {
                    read_buf = Some(buf);
                    if res? == 0 {
                        break;
                    }

                    ctx.try_decode(read_buf.as_mut().unwrap(), |req, stream_id| {
                        let s = &service;
                        queue.push(async move { (s.call(req).await, stream_id) });
                    });

                    read_task.set(read_io(read_buf.take().unwrap(), &io));
                }
                SelectOutput::B((res, id)) => {
                    let (parts, _) = match res {
                        Ok(res) => res.into_parts(),
                        Err(_) => continue,
                    };
                    let pseudo = headers::Pseudo::response(parts.status);
                    let headers = headers::Headers::new(id, pseudo, parts.headers);
                    let mut buf = write_buf.as_mut().unwrap().limit(4096);
                    headers.encode(&mut ctx.encoder, &mut buf);

                    let (res, buf) = write_io(write_buf.take().unwrap(), &io).await;
                    write_buf = Some(buf);
                    res?;
                }
            }
        }

        Ok(())
    }

    #[cold]
    #[inline(never)]
    async fn prefix_check(io: &impl AsyncBufRead, buf: &mut Option<BytesMut>) -> io::Result<()> {
        let mut b = buf.take().unwrap();
        while b.len() < PREFACE.len() {
            let (res, buf) = read_io(b, io).await;
            b = buf;
            res?;
        }

        if &b[..PREFACE.len()] == PREFACE {
            b.advance(PREFACE.len());
        } else {
            todo!()
        }

        buf.replace(b);

        Ok(())
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
