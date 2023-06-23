use core::{
    future::{pending, poll_fn, Future},
    task::Poll,
};

use alloc::{collections::VecDeque, rc::Rc};

use std::io;

use postgres_protocol::message::backend;
use xitca_io::bytes::Buf;
use xitca_io::{
    bytes::BytesMut,
    io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf},
};
use xitca_unsafe_collection::futures::{ReusableLocalBoxFuture, Select, SelectOutput};

use crate::{error::Error, iter::AsyncIterator};

use super::{
    codec::{ResponseMessage, ResponseSender},
    generic::GenericDriverRx,
};

type Opt = (io::Result<usize>, BytesMut);

// completion based io task.
// every task future take ownership of BytesMut and run to completion before the next task future
// can be made.
struct BufTask {
    buf: Option<BytesMut>,
    task: ReusableLocalBoxFuture<'static, Opt>,
}

impl BufTask {
    fn new(buf: BytesMut) -> Self {
        Self {
            buf: Some(buf),
            task: ReusableLocalBoxFuture::new(pending()),
        }
    }

    fn set_with<F, Fut>(&mut self, func: F)
    where
        F: FnOnce(BytesMut) -> Fut,
        Fut: Future<Output = Opt> + 'static,
    {
        let buf = self.buf.take().expect("BufferTask::poll must run to completion.");
        self.task.set(func(buf));
    }

    fn try_buf(&mut self) -> Option<&mut BytesMut> {
        self.buf.as_mut()
    }

    async fn poll(&mut self) -> io::Result<usize> {
        debug_assert!(
            self.buf.is_none(),
            "BufferTask must not be polled before BufferTask::set_with is called"
        );
        let (res, buf) = self.task.get_pin().await;
        self.buf = Some(buf);
        res
    }
}

pub struct IoUringDriver<Io> {
    io: Rc<Io>,
    read_task: BufTask,
    write_task: BufTask,
    rx: GenericDriverRx,
    res: VecDeque<ResponseSender>,
}

impl<Io> IoUringDriver<Io>
where
    Io: AsyncBufRead + AsyncBufWrite + 'static,
{
    #[allow(dead_code)]
    pub(crate) fn new(
        io: Io,
        rx: GenericDriverRx,
        write_buf: BytesMut,
        read_buf: BytesMut,
        res: VecDeque<ResponseSender>,
    ) -> Self {
        Self {
            io: Rc::new(io),
            read_task: BufTask::new(read_buf),
            write_task: BufTask::new(write_buf),
            rx,
            res,
        }
    }

    pub(crate) async fn try_next(&mut self) -> Result<Option<backend::Message>, Error> {
        loop {
            if let Some(buf) = self.read_task.try_buf() {
                while let Some(res) = ResponseMessage::try_from_buf(buf)? {
                    match res {
                        ResponseMessage::Normal { buf, complete } => {
                            let _ = self.res.front_mut().expect("out of bound must not happen").send(buf);
                            if complete {
                                let _ = self.res.pop_front();
                            }
                        }
                        ResponseMessage::Async(msg) => return Ok(Some(msg)),
                    }
                }

                self.read_task.set_with(|mut buf| {
                    let len = buf.len();
                    let rem = buf.capacity() - len;

                    if rem < 4096 {
                        buf.reserve(4096 - rem);
                    }

                    let io = self.io.clone();
                    async move {
                        let (res, slice) = io.read(buf.slice(len..)).await;
                        (res, slice.into_inner())
                    }
                });
            }

            let write_task = async {
                while let Some(buf) = self.write_task.try_buf() {
                    let res = poll_fn(|cx| match self.rx.poll_recv(cx) {
                        Poll::Ready(Some(req)) => Poll::Ready(SelectOutput::A(Some(req))),
                        Poll::Ready(None) if !buf.is_empty() => Poll::Ready(SelectOutput::B(())),
                        Poll::Ready(None) => Poll::Ready(SelectOutput::A(None)),
                        Poll::Pending if !buf.is_empty() => Poll::Ready(SelectOutput::B(())),
                        Poll::Pending => Poll::Pending,
                    })
                    .await;

                    match res {
                        SelectOutput::A(Some(req)) => {
                            buf.extend_from_slice(req.msg.as_ref());
                            self.res.push_back(req.tx);
                        }
                        SelectOutput::A(None) => return Ok(0),
                        SelectOutput::B(_) => {
                            self.write_task.set_with(|buf| {
                                let io = self.io.clone();
                                async move {
                                    let (res, mut buf) = io.write(buf).await;
                                    (
                                        res.map(|n| {
                                            buf.advance(n);
                                            n
                                        }),
                                        buf,
                                    )
                                }
                            });
                        }
                    }
                }

                self.write_task.poll().await
            };

            match write_task.select(self.read_task.poll()).await {
                SelectOutput::A(res) | SelectOutput::B(res) => {
                    if res? == 0 {
                        return Ok(None);
                    }
                }
            }
        }
    }
}

impl<Io> AsyncIterator for IoUringDriver<Io>
where
    Io: AsyncBufRead + AsyncBufWrite + 'static,
{
    type Future<'f> = impl Future<Output = Option<Self::Item<'f>>> + 'f where Self: 'f;
    type Item<'i> = Result<backend::Message, Error> where Self: 'i;

    #[inline]
    fn next(&mut self) -> Self::Future<'_> {
        async { self.try_next().await.transpose() }
    }
}
