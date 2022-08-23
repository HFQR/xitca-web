use std::io;

use xitca_io::{
    bytes::BytesMut,
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::{
    bytes::read_buf,
    futures::{Select as _, SelectOutput},
    uninit,
};

#[cfg(not(feature = "single-thread"))]
use tokio::sync::mpsc::{channel, Receiver};

#[cfg(feature = "single-thread")]
use xitca_unsafe_collection::channel::mpsc::{async_vec as channel, Receiver};

use crate::{
    client::Client,
    error::{unexpected_eof_err, write_zero_err, Error},
    request::Request,
    response::Response,
};

use super::context::Context;

pub struct BufferedIo<Io, const BATCH_LIMIT: usize> {
    io: Io,
    rx: Receiver<Request>,
    ctx: Context<BATCH_LIMIT>,
}

impl<Io, const BATCH_LIMIT: usize> BufferedIo<Io, BATCH_LIMIT>
where
    Io: AsyncIo,
{
    pub fn new_pair(io: Io, backlog: usize) -> (Client, Self) {
        let ctx = Context::<BATCH_LIMIT>::new();

        let (tx, rx) = channel(backlog);

        (Client::new(tx), Self { io, rx, ctx })
    }

    // send request in self blocking manner. this call would not utilize concurrent read/write nor
    // pipeline/batch. A single response is returned.
    pub async fn linear_request<F>(&mut self, encoder: F) -> Result<Response, Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        let mut buf = BytesMut::new();
        encoder(&mut buf)?;
        let msg = buf.freeze();

        let (req, res) = Request::new_pair(msg);

        self.ctx.push_req(req);

        while !self.ctx.req_is_empty() {
            self.io.ready(Interest::WRITABLE).await?;
            self.try_write()?;
        }

        loop {
            self.io.ready(Interest::READABLE).await?;
            self.try_read()?;

            if self.ctx.try_response_once()? {
                return Ok(res);
            }
        }
    }

    pub fn clear_ctx(&mut self) {
        self.ctx.clear();
    }

    // try read async io until connection error/closed/blocked.
    fn try_read(&mut self) -> Result<(), Error> {
        loop {
            match read_buf(&mut self.io, &mut self.ctx.buf) {
                Ok(0) => return Err(unexpected_eof_err()),
                Ok(_) => continue,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    // try write to async io with vectored write enabled.
    fn try_write(&mut self) -> Result<(), Error> {
        while !self.ctx.req_is_empty() {
            let mut iovs = uninit::uninit_array::<_, BATCH_LIMIT>();

            let slice = self.ctx.chunks_vectored(&mut iovs);

            match self.io.write_vectored(slice) {
                Ok(0) => return Err(write_zero_err()),
                Ok(n) => self.ctx.advance(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }
}

#[cfg(feature = "single-thread")]
mod io_impl {
    use super::*;

    use std::io::IoSlice;

    use xitca_io::bytes::Buf;
    use xitca_unsafe_collection::uninit::PartialInit;

    impl<Io, const BATCH_LIMIT: usize> BufferedIo<Io, BATCH_LIMIT>
    where
        Io: AsyncIo,
    {
        pub async fn run(mut self) -> Result<(), Error> {
            loop {
                let ready = match self.rx.wait().select(self.io.ready(Interest::READABLE)).await {
                    SelectOutput::A(res) => {
                        res.map_err(|_| unexpected_eof_err())?;
                        self.io.ready(Interest::READABLE | Interest::WRITABLE).await?
                    }
                    SelectOutput::B(ready) => ready?,
                };

                if ready.is_readable() {
                    self.try_read()?;
                    self.ctx.try_response()?;
                }

                if ready.is_writable() {
                    self.try_write2()?;
                }
            }
        }

        fn try_write2(&mut self) -> Result<(), Error> {
            let res = self.rx.with_iter(|iter| {
                let mut iovs = uninit::uninit_array::<_, BATCH_LIMIT>();
                let slice = iovs.init_from(iter).into_init_with(|req| IoSlice::new(req.msg.chunk()));
                self.io.write_vectored(slice)
            });

            match res {
                Ok(0) => Err(write_zero_err()),
                Ok(mut n) => {
                    self.rx.advance_until(|req| {
                        let rem = req.msg.remaining();

                        if rem > n {
                            req.msg.advance(n);
                            false
                        } else {
                            n -= rem;
                            self.ctx.push_res(req.tx.take().unwrap());
                            true
                        }
                    });
                    Ok(())
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
                Err(e) => Err(e.into()),
            }
        }
    }
}

#[cfg(not(feature = "single-thread"))]
mod io_impl {
    use super::*;

    impl<Io, const BATCH_LIMIT: usize> BufferedIo<Io, BATCH_LIMIT>
    where
        Io: AsyncIo,
    {
        pub async fn run(mut self) -> Result<(), Error> {
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
                        }
                    }
                }
            }

            Ok(())
        }
    }

    async fn try_rx<const BATCH_LIMIT: usize>(
        rx: &mut Receiver<Request>,
        ctx: &Context<BATCH_LIMIT>,
    ) -> Option<Request> {
        if ctx.req_is_full() {
            std::future::pending().await
        } else {
            rx.recv().await
        }
    }

    fn try_io<'i, Io, const BATCH_LIMIT: usize>(io: &'i mut Io, ctx: &Context<BATCH_LIMIT>) -> Io::ReadyFuture<'i>
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
}
