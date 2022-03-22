use std::pin::Pin;

use tokio::sync::mpsc::{channel, Receiver};
use xitca_io::io::{AsyncIo, AsyncWrite, Interest};

use super::{
    client::Client,
    context::Context,
    error::Error,
    futures::{never, poll_fn, Select, SelectOutput},
    request::Request,
};

pub(crate) struct BufferedIo<Io, const BATCH_LIMIT: usize> {
    io: Io,
    rx: Receiver<Request>,
    ctx: Context<BATCH_LIMIT>,
}

impl<Io, const BATCH_LIMIT: usize> BufferedIo<Io, BATCH_LIMIT>
where
    Io: AsyncIo,
{
    pub(crate) fn new_pair(io: Io, backlog: usize, ctx: Context<BATCH_LIMIT>) -> (Client, Self) {
        let (tx, rx) = channel(backlog);

        (Client::new(tx), Self { io, rx, ctx })
    }

    pub(crate) async fn run(self) -> Result<(), Error> {
        let Self {
            mut io,
            mut rx,
            mut ctx,
        } = self;

        loop {
            match try_rx(&mut rx, &ctx).select(try_io(&mut io, &ctx)).await {
                // batch message and keep polling.
                SelectOutput::A(Some(msg)) => ctx.push_req(msg),
                // client is gone.
                SelectOutput::A(None) => break,
                SelectOutput::B(ready) => {
                    let ready = ready?;

                    if ready.is_readable() {
                        ctx.try_read_io(&mut io)?;
                        ctx.try_response()?;
                    }

                    if ready.is_writable() {
                        ctx.try_write_io(&mut io)?;
                        poll_fn(|cx| AsyncWrite::poll_flush(Pin::new(&mut io), cx)).await?;
                    }
                }
            }
        }

        Ok(())
    }
}

async fn try_rx<const BATCH_LIMIT: usize>(rx: &mut Receiver<Request>, ctx: &Context<BATCH_LIMIT>) -> Option<Request> {
    if ctx.req_is_full() {
        never().await
    } else {
        rx.recv().await
    }
}

fn try_io<'i, Io, const BATCH_LIMIT: usize>(io: &'i mut Io, ctx: &'i Context<BATCH_LIMIT>) -> Io::ReadyFuture<'i>
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
