use std::{future::pending, io, sync::Arc};

use tokio::{
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver},
        Notify,
    },
    task::JoinHandle,
};
use xitca_io::{
    bytes::Buf,
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::{
    bytes::read_buf,
    futures::{Select as _, SelectOutput},
};

use crate::{
    client::Client,
    error::{unexpected_eof_err, write_zero_err, Error},
    request::Request,
};

use super::context::Context;

pub struct BufferedIo<Io> {
    io: Io,
    rx: UnboundedReceiver<Request>,
    ctx: Context,
}

impl<Io> BufferedIo<Io>
where
    Io: AsyncIo,
{
    pub fn new_pair(io: Io, _: usize) -> (Client, Self) {
        let ctx = Context::new();
        let (tx, rx) = unbounded_channel();
        (Client::new(tx), Self { io, rx, ctx })
    }

    pub fn clear_ctx(&mut self) {
        self.ctx.clear();
    }

    // try read async io until connection error/closed/blocked.
    fn try_read(&mut self) -> Result<(), Error> {
        loop {
            match read_buf(&mut self.io, &mut self.ctx.res_buf) {
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
            match self.io.write(&self.ctx.req_buf) {
                Ok(0) => return write_zero(self.ctx.req_is_empty()),
                Ok(n) => {
                    self.ctx.req_buf.advance(n);
                    if self.ctx.req_is_empty() {
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
            match try_rx(&mut self.rx, &self.ctx)
                .select(try_io(&mut self.io, &self.ctx))
                .await
            {
                // batch message and keep polling.
                SelectOutput::A(Some(req)) => self.ctx.push_concurrent_req(req),
                // client is gone.
                SelectOutput::A(None) => break,
                SelectOutput::B(ready) => {
                    let ready = ready?;

                    if ready.is_readable() {
                        self.try_read()?;
                        self.ctx.try_response()?;
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

async fn try_rx(rx: &mut UnboundedReceiver<Request>, ctx: &Context) -> Option<Request> {
    if ctx.throttled() {
        pending().await
    } else {
        rx.recv().await
    }
}

fn try_io<'i, Io>(io: &'i mut Io, ctx: &Context) -> Io::ReadyFuture<'i>
where
    Io: AsyncIo,
{
    let interest = if ctx.req_is_empty() {
        Interest::READABLE
    } else {
        Interest::READABLE | Interest::WRITABLE
    };

    io.ready(interest)
}

#[cold]
#[inline(never)]
fn write_zero(is_buf_empty: bool) -> Result<(), Error> {
    assert!(!is_buf_empty, "trying to write from empty buffer.");
    Err(write_zero_err())
}
