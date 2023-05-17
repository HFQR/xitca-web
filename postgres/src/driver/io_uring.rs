use core::{
    future::{pending, poll_fn, Future},
    mem,
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

// a reusable boxed future.
// it must poll one future to completion before the next one can be set.
struct Task {
    fut: ReusableLocalBoxFuture<'static, Opt>,
    can_set: bool,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            fut: ReusableLocalBoxFuture::new(pending()),
            can_set: true,
        }
    }
}

impl Task {
    fn set(&mut self, fut: impl Future<Output = Opt> + 'static) {
        assert!(
            mem::replace(&mut self.can_set, false),
            "Task must not be set multiple times before Task::poll is finished"
        );
        self.fut.set(fut);
    }

    fn can_set(&self) -> bool {
        self.can_set
    }

    async fn poll(&mut self) -> Opt {
        debug_assert!(!self.can_set(), "Task must not be polled before Task::set is called");
        let res = self.fut.get_pin().await;
        self.can_set = true;
        res
    }
}

pub struct IoUringDriver<Io> {
    io: Rc<Io>,
    read_task: Task,
    write_task: Task,
    write_buf: BytesMut,
    read_buf: BytesMut,
    rx: GenericDriverRx,
    res: VecDeque<ResponseSender>,
}

impl<Io> IoUringDriver<Io>
where
    Io: AsyncBufRead + AsyncBufWrite + 'static,
{
    pub(crate) fn new(
        io: Io,
        rx: GenericDriverRx,
        write_buf: BytesMut,
        read_buf: BytesMut,
        res: VecDeque<ResponseSender>,
    ) -> Self {
        Self {
            io: Rc::new(io),
            read_task: Task::default(),
            write_task: Task::default(),
            write_buf,
            read_buf,
            rx,
            res,
        }
    }

    pub(crate) async fn try_next(&mut self) -> Result<Option<backend::Message>, Error> {
        loop {
            if self.read_task.can_set() {
                while let Some(res) = ResponseMessage::try_from_buf(&mut self.read_buf)? {
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

                let mut buf = mem::take(&mut self.read_buf);
                let len = buf.len();
                let rem = buf.capacity() - len;

                if rem < 4096 {
                    buf.reserve(4096 - rem);
                }

                let io = self.io.clone();
                self.read_task.set(async move {
                    let (res, slice) = io.read(buf.slice(len..)).await;
                    (res, slice.into_inner())
                });
            }

            let res = self
                .read_task
                .poll()
                .select(async {
                    if self.write_task.can_set() {
                        let res = poll_fn(|cx| match self.rx.poll_recv(cx) {
                            Poll::Ready(res) => Poll::Ready(SelectOutput::A(res)),
                            Poll::Pending if !self.write_buf.is_empty() => Poll::Ready(SelectOutput::B(())),
                            Poll::Pending => Poll::Pending,
                        })
                        .await;

                        match res {
                            SelectOutput::A(req) => return SelectOutput::A(req),
                            SelectOutput::B(_) => {
                                let buf = mem::take(&mut self.write_buf);
                                let io = self.io.clone();
                                self.write_task.set(async move { io.write(buf).await });
                            }
                        }
                    }

                    let res = self.write_task.poll().await;
                    SelectOutput::B(res)
                })
                .await;

            match res {
                SelectOutput::A((res, buf)) => {
                    self.read_buf = buf;
                    let n = res?;
                    if n == 0 {
                        return Ok(None);
                    }
                }
                SelectOutput::B(SelectOutput::A(Some(req))) => {
                    self.write_buf.extend_from_slice(req.msg.as_ref());
                    self.res.push_back(req.tx);
                }
                SelectOutput::B(SelectOutput::A(None)) => return Ok(None),
                SelectOutput::B(SelectOutput::B((res, buf))) => {
                    self.write_buf = buf;
                    let n = res?;
                    if n == 0 {
                        return Ok(None);
                    }
                    self.write_buf.advance(n);
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
