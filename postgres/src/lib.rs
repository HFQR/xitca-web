#![feature(generic_associated_types, type_alias_impl_trait)]

//! A postgresql client on top of [rust-postgres](https://github.com/sfackler/rust-postgres/).

mod client;
mod config;
mod connect;
mod context;
mod futures;
mod prepare;
mod query;
mod request;
mod response;
mod row;
mod statement;

pub mod error;

pub use config::Config;
pub use row::Row;
pub use statement::Statement;

use std::{future::Future, pin::Pin};

use tokio::sync::mpsc::{channel, Receiver};
use xitca_io::io::{AsyncIo, AsyncWrite, Interest};

use crate::{
    client::Client,
    context::Context,
    error::Error,
    futures::{never, poll_fn, Select, SelectOutput},
    request::Request,
};

#[derive(Debug)]
pub struct Postgres<C, const BATCH_LIMIT: usize> {
    cfg: C,
    backlog: usize,
}

impl<C> Postgres<C, 20>
where
    Config: TryFrom<C>,
    Error: From<<Config as TryFrom<C>>::Error>,
{
    pub fn new(cfg: C) -> Self {
        Self { cfg, backlog: 128 }
    }
}

impl<C, const BATCH_LIMIT: usize> Postgres<C, BATCH_LIMIT>
where
    Config: TryFrom<C>,
    Error: From<<Config as TryFrom<C>>::Error>,
{
    pub fn backlog(mut self, num: usize) -> Self {
        self.backlog = num;
        self
    }

    pub fn batch_limit<const BATCH_LIMIT2: usize>(self) -> Postgres<C, BATCH_LIMIT2> {
        Postgres {
            cfg: self.cfg,
            backlog: self.backlog,
        }
    }

    pub async fn connect(self) -> Result<(Client, impl Future<Output = Result<(), Error>>), Error> {
        let cfg = Config::try_from(self.cfg)?;

        let mut io = crate::connect::connect(cfg).await?;

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

        Ok((Client::new(tx), fut))
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn postgres() {
        let _ = Postgres::new("abc");
        let _ = Postgres::new(Config::default());
    }
}
