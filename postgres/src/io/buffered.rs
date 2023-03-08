use core::{
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
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::{
    bytes::read_buf,
    futures::{Select as _, SelectOutput},
};

use crate::{
    error::{unexpected_eof_err, write_zero_err, Error},
    request::Request,
};

use super::context::Context;

pub struct BufferedIo<Io> {
    io: Io,
    buf_write: BufWrite,
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
            buf_write: BufWrite::default(),
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
        self.buf_write.write(&mut self.io).map_err(|e| {
            self.buf_write.reset();
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
            let want_write = self.buf_write.want_write();
            match self.rx.recv().select(try_io(&mut self.io, want_write)).await {
                // batch message and keep polling.
                SelectOutput::A(Some(req)) => {
                    self.buf_write.extend_from_slice(req.msg.as_ref());
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
                let want_write = self.buf_write.want_write();
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

fn try_io<Io>(io: &mut Io, want_write: bool) -> Io::Future<'_>
where
    Io: AsyncIo,
{
    let interest = if want_write {
        Interest::READABLE | Interest::WRITABLE
    } else {
        Interest::READABLE
    };

    io.ready(interest)
}

#[derive(Default)]
struct BufWrite {
    buf: BytesMut,
    want_flush: bool,
}

impl BufWrite {
    fn extend_from_slice(&mut self, extend: &[u8]) {
        self.buf.extend_from_slice(extend);
        // never flush when buf is not empty.
        self.want_flush = false;
    }

    fn want_write(&mut self) -> bool {
        !self.buf.is_empty() || self.want_flush
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.want_flush = false;
    }

    fn write<Io>(&mut self, io: &mut Io) -> io::Result<()>
    where
        Io: io::Write,
    {
        loop {
            if self.want_flush {
                match io.flush() {
                    Ok(_) => self.want_flush = false,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
                break;
            }

            match io.write(&self.buf) {
                Ok(0) => return Err(write_zero_err()),
                Ok(n) => {
                    self.buf.advance(n);
                    if self.buf.is_empty() {
                        // only want flush when buf becomes empty. input Io may have internal
                        // buffering rely on flush to send all data to remote.
                        // (tls buffered io for example)
                        self.want_flush = true;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }
}
