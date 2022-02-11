//! A postgresql client on top of tokio.

mod client;
mod context;
mod futures;
mod message;
mod prepare;
mod statement;

pub mod error;

pub use statement::Statement;

use std::{future::Future, io};

use tokio::sync::mpsc::{channel, Receiver};
use xitca_io::{
    io::{AsyncIo, Interest},
    net::TcpStream,
};

use crate::{
    client::Client,
    context::Context,
    futures::{never, Select, SelectOutput},
    message::Request,
};

#[derive(Debug)]
pub struct Postgres<'a, const BATCH_LIMIT: usize> {
    url: &'a str,
    backlog: usize,
}

impl<'a> Postgres<'a, 20> {
    pub fn new(url: &'a str) -> Self {
        Self { url, backlog: 128 }
    }
}

impl<'a, const BATCH_LIMIT: usize> Postgres<'a, BATCH_LIMIT> {
    pub fn backlog(mut self, num: usize) -> Self {
        self.backlog = num;
        self
    }

    pub fn batch_limit<const BATCH_LIMIT2: usize>(self) -> Postgres<'a, BATCH_LIMIT2> {
        Postgres {
            url: self.url,
            backlog: self.backlog,
        }
    }

    pub async fn connect(self) -> io::Result<(Client, impl Future<Output = Result<(), crate::error::Error>>)> {
        let stream = TcpStream::connect(self.url).await?;
        Ok(self.start_with_io(stream))
    }

    #[cfg(unix)]
    pub async fn connect_unix(self) -> io::Result<(Client, impl Future<Output = Result<(), crate::error::Error>>)> {
        let stream = xitca_io::net::UnixStream::connect(self.url).await?;
        Ok(self.start_with_io(stream))
    }

    // Start client and io task with given io type that impl `AsyncIo` trait.
    pub fn start_with_io<S>(self, mut io: S) -> (Client, impl Future<Output = Result<(), crate::error::Error>>)
    where
        S: AsyncIo,
    {
        let (tx, rx) = channel(self.backlog);

        let mut receiver = QueryReceiver::<BATCH_LIMIT> { rx };

        let mut ctx = Context::<BATCH_LIMIT>::new();

        let fut = async move {
            loop {
                match receiver
                    .recv(ctx.req_len())
                    .select(io.ready(Interest::READABLE | Interest::WRITABLE))
                    .await
                {
                    // batch message and keep polling.
                    SelectOutput::A(Some(msg)) => {
                        ctx.push_req(msg);
                    }
                    // client is gone.
                    SelectOutput::A(None) => break,
                    SelectOutput::B(ready) => {
                        let ready = ready?;

                        if ready.is_readable() {
                            // decode
                        }
                        if ready.is_writable() {
                            ctx.try_write_io(&mut io)?;
                        }
                    }
                }
            }

            Ok(())
        };

        (Client::new(tx), fut)
    }
}

struct QueryReceiver<const BATCH_LIMIT: usize> {
    rx: Receiver<Request>,
}

impl<const BATCH_LIMIT: usize> QueryReceiver<BATCH_LIMIT> {
    async fn recv(&mut self, batched: usize) -> Option<Request> {
        if batched == BATCH_LIMIT {
            never().await
        } else {
            self.rx.recv().await
        }
    }
}
