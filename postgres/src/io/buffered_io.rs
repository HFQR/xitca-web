use std::{io, pin::Pin};

use tracing::trace;
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, AsyncWrite, Interest},
};
use xitca_unsafe_collection::{
    futures::{poll_fn, Select as _, SelectOutput},
    mpsc::{async_array, Receiver},
    uninit::{self, PartialInit},
};

use crate::{client::Client, error::Error, request::Request, response::Response};

use super::context::Context;

pub struct BufferedIo<Io, const BATCH_LIMIT: usize> {
    io: Io,
    rx: Receiver<Request, 32>,
    ctx: Context<BATCH_LIMIT>,
}

impl<Io, const BATCH_LIMIT: usize> BufferedIo<Io, BATCH_LIMIT>
where
    Io: AsyncIo,
{
    pub fn new_pair(io: Io) -> (Client, Self) {
        let ctx = Context::<BATCH_LIMIT>::new();

        let (tx, rx) = async_array();

        (Client::new(tx), Self { io, rx, ctx })
    }

    // send request in self blocking manner. this call would not utilize concurrent read/write nor
    // pipeline/batch. A single response is returned.
    pub async fn linear_request<F>(&mut self, client: &Client, encoder: F) -> Result<Response, Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        let mut buf = BytesMut::new();
        encoder(&mut buf)?;
        let msg = buf.freeze();

        let (req, res) = Request::new_pair(msg);

        client.tx.send(req).await;

        while !self.rx.is_empty() {
            let _ = self.io.ready(Interest::WRITABLE).await?;
            self.try_write()?;
        }

        loop {
            let _ = self.io.ready(Interest::READABLE).await?;
            self.try_read()?;

            if self.ctx.try_response_once()? {
                return Ok(res);
            }
        }
    }

    pub fn clear_ctx(&mut self) {
        self.ctx.clear();
    }

    pub async fn run(mut self) -> Result<(), Error> {
        loop {
            let ready = match self.rx.wait().select(self.io.ready(Interest::READABLE)).await {
                SelectOutput::A(_) => self.io.ready(Interest::READABLE | Interest::WRITABLE).await?,
                SelectOutput::B(ready) => ready?,
            };

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

    // try read async io until connection error/closed/blocked.
    fn try_read(&mut self) -> Result<(), Error> {
        trace!("Starting read from IO.");

        loop {
            match self.io.try_read_buf(self.ctx.buf_mut()) {
                Ok(0) => return Err(Error::ConnectionClosed),
                Ok(_) => continue,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    // try write to async io with vectored write enabled.
    fn try_write(&mut self) -> Result<(), Error> {
        trace!("Starting write to IO.");

        let res = self.rx.with_slice(|a, b| {
            let mut iovs = uninit::uninit_array::<_, BATCH_LIMIT>();
            let slice = iovs
                .init_from(a.iter().chain(b))
                .into_init_with(|req| io::IoSlice::new(req.msg.chunk()));

            self.io.try_write_vectored(slice)
        });

        match res {
            Ok(0) => Err(Error::ConnectionClosed),
            Ok(mut n) => {
                trace!("Successful written {} bytes to IO", n);
                self.rx.advance_until(|req| {
                    let rem = req.msg.remaining();

                    if rem > n {
                        req.msg.advance(n);

                        trace!("Partial IO write operation occur. Potential IO overhead");

                        false
                    } else {
                        n -= rem;

                        trace!("Advancing {} bytes from written bytes, remaining bytes {}", rem, n);

                        self.ctx.add_pending_res(req.tx.take().unwrap());
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
