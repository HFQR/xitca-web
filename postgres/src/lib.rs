#![forbid(unsafe_code)]
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
    /// Set backlog of pending async queries.
    pub fn backlog(mut self, num: usize) -> Self {
        self.backlog = num;
        self
    }

    /// Set upper bound of batched queries.
    pub fn batch_limit<const BATCH_LIMIT2: usize>(self) -> Postgres<C, BATCH_LIMIT2> {
        Postgres {
            cfg: self.cfg,
            backlog: self.backlog,
        }
    }

    /// Connect to database. The returned values are [Client] and a detached async task
    /// that drives the client communication to db and it needs to spawn on an async runtime.
    ///
    /// # Examples:
    /// ```rust
    /// # use xitca_postgres::Postgres;
    /// # async fn connect() {
    /// let cfg = String::from("postgres://user:pass@localhost/db");
    /// let (cli, task) = Postgres::new(cfg).connect().await.unwrap();
    ///
    /// tokio::spawn(task);
    ///
    /// let stmt = cli.prepare("SELECT *", &[]).await.unwrap();
    /// # }
    ///
    /// ```
    pub async fn connect(self) -> Result<(Client, impl Future<Output = Result<(), Error>> + Send + 'static), Error> {
        let cfg = Config::try_from(self.cfg)?;
        let io = connect::connect(&cfg).await?;

        let (cli, mut io) = io::buffered_io::BufferedIo::<_, BATCH_LIMIT>::new_pair(io);

        connect::authenticate(&cli, &mut io, cfg).await?;

        // clear context before continue.
        io.clear_ctx();

        Ok((cli, io.run()))
    }
}

fn slice_iter<'a>(s: &'a [&(dyn ToSql + Sync)]) -> impl ExactSizeIterator<Item = &'a dyn ToSql> {
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
