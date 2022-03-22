use std::{io, pin::Pin};

use tokio::sync::mpsc::{channel, Receiver};
use xitca_io::{
    bytes::BytesMut,
    io::{AsyncIo, AsyncWrite, Interest},
};

use super::{
    client::Client,
    context::Context,
    error::Error,
    futures::{never, poll_fn, Select, SelectOutput},
    request::Request,
    response::Response,
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
    pub(crate) fn new_pair(io: Io, backlog: usize) -> (Client, Self) {
        let ctx = Context::<BATCH_LIMIT>::new();

        let (tx, rx) = channel(backlog);

        (Client::new(tx), Self { io, rx, ctx })
    }

    // send request in self blocking manner. this call would not utilize concurrent read/write nor
    // pipeline/batch. A single response is returned.
    pub(crate) async fn linear_request<F>(&mut self, encoder: F) -> Result<Response, Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        let mut buf = BytesMut::new();

        encoder(&mut buf)?;

        let msg = buf.freeze();

        let (req, res) = Request::new_pair(msg);

        self.ctx.push_req(req);

        while !self.ctx.req_is_empty() {
            let _ = self.io.ready(Interest::WRITABLE).await?;
            self.try_write()?;
        }

        while !self.ctx.res_is_empty() {
            let _ = self.io.ready(Interest::READABLE).await?;
            self.try_read()?;
            self.ctx.try_response()?;
        }

        Ok(res)
    }

    pub(crate) async fn run(mut self) -> Result<(), Error> {
        loop {
            match try_rx(&mut self.rx, &self.ctx)
                .select(try_io(&mut self.io, &self.ctx))
                .await
            {
                // batch message and keep polling.
                SelectOutput::A(Some(msg)) => self.ctx.push_req(msg),
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
                        poll_fn(|cx| AsyncWrite::poll_flush(Pin::new(&mut self.io), cx)).await?;
                    }
                }
            }
        }

        Ok(())
    }

    // try read async io until connection error/closed/blocked.
    fn try_read(&mut self) -> Result<(), Error> {
        assert!(
            !self.ctx.res_is_empty(),
            "If server return anything their must at least one response waiting."
        );

        loop {
            match self.io.try_read_buf(&mut self.ctx.buf) {
                Ok(0) => return Err(Error::ConnectionClosed),
                Ok(_) => continue,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    // try write to async io with vectored write enabled.
    fn try_write(&mut self) -> Result<(), Error> {
        while !self.ctx.req_is_empty() {
            let mut iovs = [io::IoSlice::new(&[]); BATCH_LIMIT];
            let len = self.ctx.chunks_vectored(&mut iovs);
            match self.io.try_write_vectored(&iovs[..len]) {
                Ok(0) => return Err(Error::ConnectionClosed),
                Ok(n) => self.ctx.advance(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
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
