#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]

//! A postgresql client on top of [rust-postgres](https://github.com/sfackler/rust-postgres/).

mod client;
mod config;
mod connect;
mod context;
mod futures;
mod io;
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

use std::future::Future;

use crate::{client::Client, error::Error};

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

        let io = crate::connect::connect(&cfg).await?;

        let (cli, mut io) = crate::io::BufferedIo::<_, BATCH_LIMIT>::new_pair(io, self.backlog);

        crate::connect::authenticate(&mut io, cfg).await?;

        Ok((cli, io.run()))
    }
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
