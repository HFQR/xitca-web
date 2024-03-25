#![forbid(unsafe_code)]

//! A postgresql client on top of [rust-postgres](https://github.com/sfackler/rust-postgres/).

mod client;
mod column;
mod config;
mod driver;
mod from_sql;
mod iter;
mod pool;
mod prepare;
mod query;
mod session;
mod util;

pub mod error;
pub mod pipeline;
pub mod row;
pub mod statement;

#[cfg(not(feature = "quic"))]
mod transaction;

#[cfg(feature = "quic")]
pub mod proxy;

pub use postgres_types::{BorrowToSql, FromSql, ToSql, Type};

pub use self::{
    client::Client,
    config::Config,
    driver::Driver,
    error::Error,
    from_sql::FromSqlExt,
    iter::AsyncLendingIterator,
    pool::SharedClient,
    query::{RowSimpleStream, RowStream},
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
        let mut cfg = Config::try_from(self.cfg)?;
        driver::connect(&mut cfg).await
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

#[cfg(all(feature = "tls", feature = "quic"))]
#[cfg(test)]
mod test {
    use std::{future::IntoFuture, sync::Arc};

    use quinn::ServerConfig;
    use rustls_0dot21::{Certificate, PrivateKey};

    use crate::{proxy::Proxy, AsyncLendingIterator, Postgres};

    #[tokio::test]
    async fn proxy() {
        let name = vec!["127.0.0.1".to_string(), "localhost".to_string()];
        let cert = rcgen::generate_simple_self_signed(name).unwrap();

        let key = PrivateKey(cert.serialize_private_key_der());
        let cert = vec![Certificate(cert.serialize_der().unwrap())];

        let mut config = rustls_0dot21::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(cert, key)
            .unwrap();

        config.alpn_protocols = vec![b"quic".to_vec()];
        let config = ServerConfig::with_crypto(Arc::new(config));

        let upstream = tokio::net::lookup_host("localhost:5432").await.unwrap().next().unwrap();

        tokio::spawn(
            Proxy::with_config(config)
                .upstream_addr(upstream)
                .listen_addr("127.0.0.1:5435".parse().unwrap())
                .run(),
        );

        let (cli, task) =
            Postgres::new("postgres://postgres:postgres@127.0.0.1:5435/postgres?target_session_attrs=read-write")
                .connect()
                .await
                .unwrap();

        let handle = tokio::spawn(task.into_future());

        let _ = cli.query_simple("").await.unwrap().try_next().await;

        drop(cli);

        handle.await.unwrap();
    }
}
