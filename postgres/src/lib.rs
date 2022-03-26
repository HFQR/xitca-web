#![feature(generic_associated_types, type_alias_impl_trait)]

//! A postgresql client on top of [rust-postgres](https://github.com/sfackler/rust-postgres/).

mod client;
mod config;
mod connect;
mod io;
mod prepare;
mod query;
mod request;
mod response;
mod row;
mod statement;

pub mod error;

pub use self::client::Client;
pub use self::config::Config;
pub use self::row::Row;
pub use self::statement::Statement;

pub use postgres_types::ToSql;

use std::future::Future;

use crate::error::Error;

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

    pub async fn connect(
        self,
    ) -> Result<
        (
            Client,
            // TODO: This is a rust compiler bug.
            // if impl Future opaque type used the 'static lifetime would be removed from trait bound.
            std::pin::Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'static>>,
        ),
        Error,
    > {
        let cfg = Config::try_from(self.cfg)?;

        let io = crate::connect::connect(&cfg).await?;

        let (cli, mut io) = crate::io::buffered_io::BufferedIo::<_, BATCH_LIMIT>::new_pair(io, self.backlog);

        crate::connect::authenticate(&mut io, cfg).await?;

        // clear context before continue.
        io.clear_ctx();

        Ok((cli, Box::pin(io.run())))
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
