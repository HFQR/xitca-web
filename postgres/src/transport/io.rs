use core::{
    convert::Infallible,
    future::{pending, poll_fn, Future},
    pin::Pin,
};

use alloc::collections::VecDeque;

use std::io;

use postgres_protocol::message::backend;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::error;
use xitca_io::{
    bytes::{BufInterest, BufRead, BufWrite, WriteBuf},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::futures::{Select as _, SelectOutput};

use crate::{error::Error, iter::AsyncIterator};

use super::codec::{Request, ResponseMessage, ResponseSender};

pub struct BufferedIo<Io> {
    io: Io,
    write_buf: WriteBuf,
    read_buf: PagedBytesMut,
    rx: Option<UnboundedReceiver<Request>>,
    res: VecDeque<ResponseSender>,
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
        rx: Some(rx),
        res: VecDeque::new(),
    }
}

impl<Io> BufferedIo<Io>
where
    Io: AsyncIo + Send + 'static,
    for<'f> Io::Future<'f>: Send,
{
    pub(crate) async fn run(mut self) -> Result<(), Error> {
        self._run().await
    }

    fn try_read(&mut self) -> Result<(), Error> {
        self.read_buf.do_io(&mut self.io).map_err(Into::into)
    }

    fn try_write(&mut self) -> io::Result<()> {
        self.write_buf.do_io(&mut self.io).map_err(|e| {
            self.write_buf.clear();
            error!("server closed connection unexpectedly: {e}");
            e
        })
    }

    fn try_decode(&mut self) -> Result<Option<backend::Message>, Error> {
        while let Some(res) = ResponseMessage::try_from_buf(&mut self.read_buf)? {
            match res {
                ResponseMessage::Normal { buf, complete } => {
                    let _ = self.res.front_mut().expect("Out of bound must not happen").send(buf);

                    if complete {
                        let _ = self.res.pop_front();
                    }
                }
                ResponseMessage::Async(msg) => return Ok(Some(msg)),
            }
        }

        Ok(None)
    }

    async fn _run(&mut self) -> Result<(), Error> {
        while self._next().await?.is_some() {}
        Ok(())
    }

    async fn _next(&mut self) -> Result<Option<backend::Message>, Error> {
        loop {
            if let Some(msg) = self.try_decode()? {
                return Ok(Some(msg));
            }

            let interest = if self.write_buf.want_write_io() {
                Interest::READABLE | Interest::WRITABLE
            } else {
                Interest::READABLE
            };

            let select = match self.rx {
                Some(ref mut rx) => {
                    let ready = self.io.ready(interest);
                    rx.recv().select(ready).await
                }
                None => {
                    if !interest.is_writable() && self.res.is_empty() {
                        // no interest to write to io and all response have been finished so
                        // shutdown io and exit.
                        // if there is a better way to exhaust potential remaining backend message
                        // please file an issue.
                        poll_fn(|cx| Pin::new(&mut self.io).poll_shutdown(cx)).await?;
                        return Ok(None);
                    }
                    let ready = self.io.ready(interest);
                    pending().select(ready).await
                }
            };

            match select {
                // batch message and keep polling.
                SelectOutput::A(Some(req)) => {
                    let _ = self.write_buf.write_buf(|buf| {
                        buf.extend_from_slice(req.msg.as_ref());
                        Ok::<_, Infallible>(())
                    });
                    if let Some(tx) = req.tx {
                        self.res.push_back(tx);
                    }
                }
                SelectOutput::B(ready) => {
                    let ready = ready?;
                    if ready.is_readable() {
                        self.try_read()?;
                    }
                    if ready.is_writable() && self.try_write().is_err() {
                        // write failed as server stopped reading.
                        // drop channel so all pending request can be notified.
                        self.rx = None;
                    }
                }
                SelectOutput::A(None) => self.rx = None,
            }
        }
    }
}

impl<Io> AsyncIterator for BufferedIo<Io>
where
    Io: AsyncIo + Send + 'static,
    for<'f> Io::Future<'f>: Send,
{
    type Future<'f> = impl Future<Output = Option<Self::Item<'f>>> + Send + 'f where Self: 'f;
    type Item<'i> = Result<backend::Message, Error> where Self: 'i;

    #[inline]
    fn next(&mut self) -> Self::Future<'_> {
        async { self._next().await.transpose() }
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
