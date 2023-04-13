#![forbid(unsafe_code)]
#![feature(impl_trait_in_assoc_type)]

//! A postgresql client on top of [rust-postgres](https://github.com/sfackler/rust-postgres/).

extern crate alloc;

mod client;
mod column;
mod config;
mod driver;
mod from_sql;
mod iter;
mod prepare;
mod query;
mod row;
mod session;
mod util;

pub mod error;
pub mod statement;

#[cfg(feature = "quic")]
pub mod proxy;

pub use postgres_types::{FromSql, ToSql, Type};

pub use self::{
    client::Client,
    config::Config,
    driver::Driver,
    error::Error,
    from_sql::FromSqlExt,
    iter::AsyncIterator,
    row::{Row, RowSimple},
};

#[derive(Debug)]
pub struct Postgres<C, const BATCH_LIMIT: usize> {
    cfg: C,
    backlog: usize,
}

impl<C> Postgres<C, 32>
where
    Config: TryFrom<C>,
    Error: From<<Config as TryFrom<C>>::Error>,
{
    pub fn new(cfg: C) -> Self {
        Self { cfg, backlog: 32 }
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
    /// use std::future::IntoFuture;
    /// use xitca_postgres::Postgres;
    /// # async fn connect() {
    /// let cfg = String::from("postgres://user:pass@localhost/db");
    /// let (client, driver) = Postgres::new(cfg).connect().await.unwrap();
    ///
    /// // spawn driver as async task.
    /// tokio::spawn(driver.into_future());
    ///
    /// // use client for query.
    /// let stmt = client.prepare("SELECT *", &[]).await.unwrap();
    /// # }
    ///
    /// ```
    pub async fn connect(self) -> Result<(Client, Driver), Error> {
        let cfg = Config::try_from(self.cfg)?;
        driver::connect(cfg).await
    }
}

fn _assert_send<F: Send>(_: F) {}
fn _assert_send2<F: Send>() {}

#[cfg(not(feature = "single-thread"))]
fn _assert_connect_send() {
    _assert_send(crate::Postgres::new("postgres://postgres:postgres@localhost/postgres").connect());
}

fn _assert_driver_send() {
    _assert_send2::<Driver>();
}

// #[cfg(not(feature = "quic"))]
// #[cfg(test)]
// mod test {
//     use crate::{AsyncIterator, Postgres};
//
//     #[tokio::test]
//     async fn postgres() {
//         let (cli, mut task) = Postgres::new("postgres://postgres:postgres@localhost/postgres")
//             .connect()
//             .await
//             .unwrap();
//         tokio::spawn(async move { while task.next().await.is_some() {} });
//         let _ = cli.query_simple("").await.unwrap().next().await;
//     }
// }
