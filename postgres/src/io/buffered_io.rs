use std::{future::pending, io, sync::Arc};

use tokio::{
    sync::{mpsc::UnboundedReceiver, Notify},
    task::JoinHandle,
};
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
    write_buf: BytesMut,
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
            write_buf: BytesMut::new(),
            read_buf: BytesMut::new(),
            rx,
            ctx: Context::new(),
        }
    }

    // try read async io until connection error/closed/blocked.
    fn try_read(&mut self) -> Result<(), Error> {
        loop {
            match read_buf(&mut self.io, &mut self.read_buf) {
                Ok(0) => return Err(unexpected_eof_err()),
                Ok(_) => continue,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    // try write to async io with vectored write enabled.
    fn try_write(&mut self) -> Result<(), Error> {
        loop {
            match self.io.write(&self.write_buf) {
                Ok(0) => return Err(write_zero_err()),
                Ok(n) => {
                    self.write_buf.advance(n);
                    if self.write_buf.is_empty() {
                        break;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

    pub async fn run(mut self) -> Result<(), Error> {
        self._run().await
    }

    pub(crate) fn spawn_run(mut self) -> (JoinHandle<Self>, Arc<Notify>)
    where
        Io: Send + 'static,
        for<'r> <Io as AsyncIo>::ReadyFuture<'r>: Send,
    {
        let notify = Arc::new(Notify::new());
        let notify2 = notify.clone();
        let handle = tokio::task::spawn(async move {
            let _ = self._run().select(notify2.notified()).await;
            self
        });
        (handle, notify)
    }

    async fn _run(&mut self) -> Result<(), Error> {
        loop {
            let want_write = !self.write_buf.is_empty();
            match try_rx(&mut self.rx, &mut self.ctx)
                .select(try_io(&mut self.io, want_write))
                .await
            {
                // batch message and keep polling.
                SelectOutput::A(Some(req)) => {
                    self.write_buf.extend_from_slice(req.msg.as_ref());
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
                        self.ctx.try_decode(&mut self.read_buf)?;
                    }
                    if ready.is_writable() {
                        self.try_write()?;
                    }
                }
            }
        }

        Ok(())
    }
}

async fn try_rx(rx: &mut UnboundedReceiver<Request>, ctx: &mut Context) -> Option<Request> {
    if ctx.throttled() {
        pending().await
    } else {
        rx.recv().await
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
