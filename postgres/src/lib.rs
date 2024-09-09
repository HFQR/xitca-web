//! A postgresql client on top of [`rust-postgres`](https://github.com/sfackler/rust-postgres/)
//!
//! This crate shares a similar feature set and public API with [`tokio-postgres`](https://docs.rs/tokio-postgres/latest/tokio_postgres/) with some differences:
//!
//! # Pipelining
//!
//! offer both "implicit" and explicit API. support for more relaxed pipeline.
//!
//! # SSL/TLS support
//!
//! powered by `rustls`
//!
//! # QUIC transport layer
//!
//! offer transparent QUIC transport layer and proxy for lossy remote database connection
//!
//! # Connection Pool
//!
//! built in connection pool with pipelining support enabled

#![forbid(unsafe_code)]

mod cancel;
mod client;
mod column;
mod config;
mod driver;
mod from_sql;
mod iter;
mod prepare;
mod query;
mod session;
mod transaction;

pub mod error;
pub mod pipeline;
pub mod pool;
pub mod row;
pub mod statement;

#[cfg(feature = "quic")]
pub mod proxy;
#[cfg(feature = "quic")]
pub use driver::quic::QuicStream;

pub use postgres_types::{BorrowToSql, FromSql, ToSql, Type};

pub use self::{
    client::Client,
    config::Config,
    driver::Driver,
    error::Error,
    from_sql::FromSqlExt,
    iter::AsyncLendingIterator,
    query::{RowSimpleStream, RowStream},
    session::Session,
};

use core::{future::Future, pin::Pin};

use xitca_io::io::AsyncIo;

#[derive(Debug)]
pub struct Postgres {
    cfg: Result<Config, Error>,
}

impl Postgres {
    pub fn new<C>(cfg: C) -> Self
    where
        Config: TryFrom<C>,
        Error: From<<Config as TryFrom<C>>::Error>,
    {
        Self {
            cfg: Config::try_from(cfg).map_err(Into::into),
        }
    }
}

impl Postgres {
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
        let mut cfg = self.cfg?;
        driver::connect(&mut cfg).await
    }

    /// Connect to database with an already established Io type.
    /// Io type must impl [AsyncIo] trait to instruct the client and driver how to transmit
    /// data through the Io.
    pub async fn connect_io<Io>(self, io: Io) -> Result<(Client, Driver), Error>
    where
        Io: AsyncIo + Send + 'static,
    {
        let mut cfg = self.cfg?;
        driver::connect_io(io, &mut cfg).await
    }

    #[cfg(feature = "quic")]
    pub async fn connect_quic(self) -> Result<(Client, Driver), Error> {
        use config::Host;

        let mut cfg = self.cfg?;
        cfg.host = cfg
            .host
            .into_iter()
            .map(|host| match host {
                Host::Tcp(host) => Host::Quic(host),
                host => host,
            })
            .collect();
        driver::connect(&mut cfg).await
    }
}

type BoxedFuture<'a, O> = Pin<Box<dyn Future<Output = O> + Send + 'a>>;

fn _assert_send<F: Send>(_: F) {}
fn _assert_send2<F: Send>() {}

fn _assert_connect_send() {
    _assert_send(crate::Postgres::new("postgres://postgres:postgres@localhost/postgres").connect());
}

fn _assert_driver_send() {
    _assert_send2::<Driver>();
}

// temporary disabled test due to cargo workspace test bug.
// #[cfg(feature = "quic")]
// #[cfg(test)]
// mod test {
//     use std::future::IntoFuture;

//     use quinn::{rustls::pki_types::PrivatePkcs8KeyDer, ServerConfig};

//     use crate::{proxy::Proxy, AsyncLendingIterator, Config, Postgres, QuicStream};

//     #[tokio::test]
//     async fn proxy() {
//         let name = vec!["127.0.0.1".to_string(), "localhost".to_string()];
//         let cert = rcgen::generate_simple_self_signed(name).unwrap();

//         let key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into();
//         let cert = cert.cert.der().clone();

//         let mut cfg = xitca_tls::rustls::ServerConfig::builder()
//             .with_no_client_auth()
//             .with_single_cert(vec![cert], key)
//             .unwrap();

//         cfg.alpn_protocols = vec![crate::driver::quic::QUIC_ALPN.to_vec()];

//         let cfg = quinn::crypto::rustls::QuicServerConfig::try_from(cfg).unwrap();

//         let config = ServerConfig::with_crypto(std::sync::Arc::new(cfg));

//         let upstream = tokio::net::lookup_host("localhost:5432").await.unwrap().next().unwrap();

//         tokio::spawn(
//             Proxy::with_config(config)
//                 .upstream_addr(upstream)
//                 .listen_addr("127.0.0.1:5432".parse().unwrap())
//                 .run(),
//         );

//         let mut cfg = Config::new();

//         cfg.dbname("postgres").user("postgres").password("postgres");

//         let conn = crate::driver::quic::_connect_quic("127.0.0.1", &[5432]).await.unwrap();

//         let stream = conn.open_bi().await.unwrap();

//         let (cli, task) = Postgres::new(cfg).connect_io(QuicStream::from(stream)).await.unwrap();

//         let handle = tokio::spawn(task.into_future());

//         let _ = cli.query_simple("").await.unwrap().try_next().await;

//         drop(cli);

//         handle.await.unwrap();
//     }
// }
