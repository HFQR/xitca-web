use std::io;

use xitca_io::{
    bytes::{Buf, BytesMut},
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
    response::{Response, ResponseMessage},
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
    pub async fn linear_request<F, E>(&mut self, encoder: F) -> Result<Response, Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), E>,
        Error: From<E>,
    {
        let mut buf = BytesMut::new();
        encoder(&mut buf)?;

        while !buf.is_empty() {
            match self.io.write(&buf) {
                Ok(0) => return Err(Error::ToDo),
                Ok(n) => buf.advance(n),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock) => {
                    self.io.ready(Interest::WRITABLE).await?;
                }
                Err(e) => return Err(e.into()),
            }
        }

        let (mut req, res) = Request::new_pair(BytesMut::new());

        loop {
            match read_buf(&mut self.io, &mut buf) {
                Ok(0) => return Err(Error::ToDo),
                Ok(_) => {
                    if let Some(msg) = ResponseMessage::try_from_buf(&mut buf)? {
                        match msg {
                            ResponseMessage::Normal { buf, complete } => {
                                req.tx.as_mut().unwrap().send(buf).await.unwrap();
                                if complete {
                                    return Ok(res);
                                }
                            }
                            ResponseMessage::Async(_) => unreachable!("async message handling is not implemented"),
                        }
                    }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock) => {
                    self.io.ready(Interest::READABLE).await?;
                }
                Err(e) => return Err(e.into()),
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
            self.rx.with_vec(|v| {
                while !v.is_empty() {
                    let mut iovs = uninit::uninit_array::<_, BATCH_LIMIT>();
                    let slice = iovs
                        .init_from(v.iter())
                        .into_init_with(|req| IoSlice::new(req.msg.chunk()));

                    match self.io.write_vectored(slice) {
                        Ok(0) => return Err(write_zero_err()),
                        Ok(mut n) => {
                            v.advance_until(|req| {
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
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) => return Err(e.into()),
                    }
                }

                Ok(())
            })
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
