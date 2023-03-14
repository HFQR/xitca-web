use core::{
    convert::Infallible,
    future::{poll_fn, Future},
    pin::Pin,
};

use alloc::sync::Arc;

use std::io;

use tokio::{
    sync::{mpsc::UnboundedReceiver, Notify},
    task::JoinHandle,
};
use tracing::error;
use xitca_io::{
    bytes::{BufInterest, BufWrite, WriteBuf},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::{
    bytes::read_buf,
    futures::{Select as _, SelectOutput},
};

use crate::{
    error::{unexpected_eof_err, Error},
    request::Request,
};

use super::context::Context;

pub struct BufferedIo<Io> {
    io: Io,
    write_buf: WriteBuf,
    read_buf: PagedBytesMut,
    rx: UnboundedReceiver<Request>,
    ctx: Context,
}

pub(crate) type PagedBytesMut = xitca_unsafe_collection::bytes::PagedBytesMut<4096>;

impl<Io> BufferedIo<Io>
where
    Io: AsyncIo + Send + 'static,
{
    pub(crate) fn new(io: Io, rx: UnboundedReceiver<Request>) -> Self {
        Self {
            io,
            write_buf: WriteBuf::new(),
            read_buf: PagedBytesMut::new(),
            rx,
            ctx: Context::new(),
        }
    }

    fn try_read(&mut self) -> Result<(), Error> {
        loop {
            match read_buf(&mut self.io, &mut self.read_buf) {
                Ok(0) => return Err(Error::from(unexpected_eof_err())),
                Ok(_) => self.ctx.try_decode(&mut self.read_buf)?,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn try_write(&mut self) -> io::Result<()> {
        self.write_buf.write_io(&mut self.io).map_err(|e| {
            self.write_buf.clear();
            error!("server closed connection unexpectedly: {e}");
            e
        })
    }

    pub async fn run(mut self) -> Result<(), Error>
    where
        for<'r> Io::Future<'r>: Send,
    {
        self._run().await
    }

    pub(crate) fn spawn(mut self) -> Handle<Self>
    where
        for<'r> Io::Future<'r>: Send,
    {
        let notify = Arc::new(Notify::new());
        let notify2 = notify.clone();
        let handle = tokio::task::spawn(async move {
            let _ = self._run().select(notify2.notified()).await;
            self
        });
        Handle { handle, notify }
    }

    async fn _run(&mut self) -> Result<(), Error>
    where
        for<'r> Io::Future<'r>: Send,
    {
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
    fn shutdown(&mut self) -> impl Future<Output = Result<(), Error>> + Send + '_
    where
        for<'r> Io::Future<'r>: Send,
    {
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
