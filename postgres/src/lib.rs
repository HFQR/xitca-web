#![feature(generic_associated_types, type_alias_impl_trait)]

//! A postgresql client on top of tokio.

mod client;
mod context;
mod futures;
mod prepare;
mod query;
mod request;
mod response;
mod row;
mod statement;

pub mod error;

pub use row::Row;
pub use statement::Statement;

use std::{future::Future, io, pin::Pin};

use tokio::sync::mpsc::{channel, Receiver};
use xitca_io::{
    io::{AsyncIo, AsyncWrite, Interest},
    net::TcpStream,
};

use crate::{
    client::Client,
    context::Context,
    futures::{never, poll_fn, Select, SelectOutput},
    request::Request,
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

        let mut receiver = QueryReceiver { rx };

        let mut ctx = Context::<BATCH_LIMIT>::new();

        let fut = async move {
            loop {
                match receiver
                    .recv(ctx.req_is_full())
                    .select(io_ready(&mut io, !ctx.req_is_empty()))
                    .await
                {
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
        };

        (Client::new(tx), fut)
    }
}

struct QueryReceiver {
    rx: Receiver<Request>,
}

impl QueryReceiver {
    async fn recv(&mut self, req_is_full: bool) -> Option<Request> {
        if req_is_full {
            never().await
        } else {
            self.rx.recv().await
        }
    }
}

fn io_ready<Io: AsyncIo>(io: &mut Io, can_write: bool) -> Io::ReadyFuture<'_> {
    let interest = if can_write {
        Interest::READABLE | Interest::WRITABLE
    } else {
        Interest::READABLE
    };

    io.ready(interest)
}

fn slice_iter<'a>(
    s: &'a [&'a (dyn postgres_types::ToSql + Sync)],
) -> impl ExactSizeIterator<Item = &'a dyn postgres_types::ToSql> + 'a {
    s.iter().map(|s| *s as _)
}
