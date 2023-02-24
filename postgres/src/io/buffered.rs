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
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest, Ready},
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
    read_buf: BytesMut,
    rx: UnboundedReceiver<Request>,
    ctx: Context,
}

impl<Io> BufferedIo<Io>
where
    Io: AsyncIo,
{
    pub(crate) fn new(io: Io, rx: UnboundedReceiver<Request>) -> Self {
        Self {
            io,
            buf_write: BufWrite::default(),
            read_buf: BytesMut::new(),
            rx,
            ctx: Context::new(),
        }
    }

    fn handle_io(&mut self, ready: Ready) -> Result<(), Error> {
        if ready.is_readable() {
            loop {
                match read_buf(&mut self.io, &mut self.read_buf) {
                    Ok(0) => return Err(unexpected_eof_err()),
                    Ok(_) => continue,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.into()),
                }
            }
            self.ctx.try_decode(&mut self.read_buf)?;
        }

        if ready.is_writable() {
            self.buf_write.write(&mut self.io)?;
        }

        Ok(())
    }

    pub async fn run(mut self) -> Result<(), Error> {
        self._run().await
    }

    pub(crate) fn spawn(mut self) -> Handle<Self>
    where
        Io: Send + 'static,
        for<'r> Io::ReadyFuture<'r>: Send,
    {
        let notify = Arc::new(Notify::new());
        let notify2 = notify.clone();
        let handle = tokio::task::spawn(async move {
            let _ = self._run().select(notify2.notified()).await;
            self
        });
        Handle { handle, notify }
    }

    async fn _run(&mut self) -> Result<(), Error> {
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
                    self.handle_io(ready)?;
                }
            }
        }

        self.shutdown().await
    }

    #[cold]
    #[inline(never)]
    fn shutdown(&mut self) -> impl Future<Output = Result<(), Error>> + '_ {
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
                self.handle_io(ready)?;
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

fn try_io<Io>(io: &mut Io, want_write: bool) -> Io::ReadyFuture<'_>
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

    fn write<Io>(&mut self, io: &mut Io) -> Result<(), Error>
    where
        Io: io::Write,
    {
        loop {
            if self.want_flush {
                match io.flush() {
                    Ok(_) => self.want_flush = false,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
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
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }
}
