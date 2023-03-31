use core::{
    convert::Infallible,
    future::{poll_fn, Future},
    pin::Pin,
};

use alloc::collections::VecDeque;

use std::io;

use tokio::sync::mpsc::UnboundedReceiver;
use tracing::error;
use xitca_io::{
    bytes::{BufInterest, BufWrite, WriteBuf},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::{
    bytes::read_buf,
    futures::{Select as _, SelectOutput},
};

use crate::error::{unexpected_eof_err, Error};

use super::codec::{Request, ResponseMessage, ResponseSender};

pub struct BufferedIo<Io> {
    io: Io,
    write_buf: WriteBuf,
    read_buf: PagedBytesMut,
    rx: UnboundedReceiver<Request>,
    ctx: Context,
}

pub(crate) type PagedBytesMut = xitca_unsafe_collection::bytes::PagedBytesMut<4096>;

pub(crate) fn new<Io>(io: Io, rx: UnboundedReceiver<Request>) -> BufferedIo<Io>
where
    Io: AsyncIo + Send + 'static,
{
    BufferedIo {
        io,
        write_buf: WriteBuf::new(),
        read_buf: PagedBytesMut::new(),
        rx,
        ctx: Context::new(),
    }
}

impl<Io> BufferedIo<Io>
where
    Io: AsyncIo + Send + 'static,
    for<'f> Io::Future<'f>: Send,
{
    fn try_read(&mut self) -> Result<(), Error> {
        let len = self.read_buf.len();
        loop {
            match read_buf(&mut self.io, &mut self.read_buf) {
                Ok(0) => {
                    if self.read_buf.len() == len {
                        return Err(Error::from(unexpected_eof_err()));
                    }
                    break;
                }
                Ok(_) => {}
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(Error::from(e)),
            }
        }
        self.ctx.try_decode(&mut self.read_buf)
    }

    fn try_write(&mut self) -> io::Result<()> {
        self.write_buf.write_io(&mut self.io).map_err(|e| {
            self.write_buf.clear();
            error!("server closed connection unexpectedly: {e}");
            e
        })
    }

    pub(crate) async fn run(mut self) -> Result<(), Error> {
        self._run().await
    }

    async fn _run(&mut self) -> Result<(), Error> {
        loop {
            let interest = if self.write_buf.want_write_io() {
                Interest::READABLE | Interest::WRITABLE
            } else {
                Interest::READABLE
            };
            let ready = self.io.ready(interest);
            match self.rx.recv().select(ready).await {
                // batch message and keep polling.
                SelectOutput::A(Some(req)) => {
                    let _ = self.write_buf.write_buf(|buf| {
                        buf.extend_from_slice(req.msg.as_ref());
                        Ok::<_, Infallible>(())
                    });
                    if let Some(tx) = req.tx {
                        self.ctx.push_concurrent_req(tx);
                    }
                }
                // client is gone.
                SelectOutput::A(None) => break,
                SelectOutput::B(ready) => {
                    let ready = ready?;
                    if ready.is_readable() {
                        self.try_read()?;
                    }
                    if ready.is_writable() && self.try_write().is_err() {
                        break;
                    }
                }
            }
        }

        self.shutdown().await
    }

    #[cold]
    #[inline(never)]
    fn shutdown(&mut self) -> impl Future<Output = Result<(), Error>> + Send + '_ {
        Box::pin(async {
            loop {
                let want_write = self.write_buf.want_write_io();
                let want_read = !self.ctx.is_empty();
                let interest = match (want_read, want_write) {
                    (false, false) => break,
                    (true, true) => Interest::READABLE | Interest::WRITABLE,
                    (true, false) => Interest::READABLE,
                    (false, true) => Interest::WRITABLE,
                };
                let fut = self.io.ready(interest);
                let ready = fut.await?;
                if ready.is_readable() {
                    self.try_read()?;
                }
                if ready.is_writable() {
                    let _ = self.try_write();
                }
            }

            poll_fn(|cx| Pin::new(&mut self.io).poll_shutdown(cx))
                .await
                .map_err(Into::into)
        })
    }
}

pub(super) struct Context {
    concurrent_res: VecDeque<ResponseSender>,
}

impl Context {
    pub(super) fn new() -> Self {
        Self {
            concurrent_res: VecDeque::new(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.concurrent_res.is_empty()
    }

    pub(super) fn push_concurrent_req(&mut self, tx: ResponseSender) {
        self.concurrent_res.push_back(tx);
    }

    pub(super) fn try_decode(&mut self, buf: &mut PagedBytesMut) -> Result<(), Error> {
        while let Some(res) = ResponseMessage::try_from_buf(buf)? {
            if let ResponseMessage::Normal { buf, complete } = res {
                let _ = self
                    .concurrent_res
                    .front_mut()
                    .expect("Out of bound must not happen")
                    .send(buf);

                if complete {
                    let _ = self.concurrent_res.pop_front();
                }
            }
        }

        Ok(())
    }
}

#[cfg(not(feature = "quic"))]
mod raw_impl {
    use alloc::sync::Arc;

    use tokio::{sync::Notify, task::JoinHandle};
    use xitca_io::io::AsyncIo;
    use xitca_unsafe_collection::futures::Select;

    use super::BufferedIo;

    impl<Io> BufferedIo<Io>
    where
        Io: AsyncIo + Send + 'static,
        for<'f> Io::Future<'f>: Send,
    {
        pub(crate) fn spawn(mut self) -> Handle<Self> {
            let notify = Arc::new(Notify::new());
            let notify2 = notify.clone();
            let handle = tokio::spawn(async move {
                let _ = self._run().select(notify2.notified()).await;
                self
            });
            Handle { handle, notify }
        }
    }

    pub(crate) struct Handle<Io> {
        handle: JoinHandle<Io>,
        notify: Arc<Notify>,
    }

    impl<Io> Handle<Io> {
        pub(crate) async fn into_inner(self) -> Io {
            self.notify.notify_waiters();
            self.handle.await.unwrap()
        }
    }
}
