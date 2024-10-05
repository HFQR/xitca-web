#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod cancel;
mod client;
mod column;
mod config;
mod driver;
mod execute;
mod from_sql;
mod prepare;
mod query;
mod session;

pub mod copy;
pub mod error;
pub mod iter;
pub mod pipeline;
pub mod pool;
pub mod row;
pub mod statement;
pub mod transaction;
pub mod types;

#[cfg(feature = "quic")]
pub mod proxy;
#[cfg(feature = "quic")]
pub use driver::quic::QuicStream;

pub use self::{
    client::Client,
    config::Config,
    driver::Driver,
    error::Error,
    execute::{Execute, ExecuteBlocking},
    from_sql::FromSqlExt,
    query::{RowSimpleStream, RowSimpleStreamOwned, RowStream, RowStreamOwned},
    session::Session,
    statement::Statement,
};

#[cfg(feature = "compat")]
pub mod compat {
    //! compatibility mod to work with [`futures::Stream`] trait.
    //!
    //! by default this crate uses an async lending iterator to enable zero copy database row handling when possible.
    //! but this approach can be difficult to hook into existing libraries and crates. In case a traditional async iterator
    //! is needed this module offers types as adapters.
    //!
    //! # Examples
    //! ```
    //! # use xitca_postgres::{Client, Error, Execute, Statement};
    //! # async fn convert(client: Client) -> Result<(), Error> {
    //! // prepare a statement and query for rows.
    //! let stmt = Statement::named("SELECT * from users", &[]).execute(&client).await?;
    //! let mut stream = stmt.query(&client).await?;
    //!
    //! // assuming we want to spawn a tokio async task and handle the stream inside it.
    //! // but code below would not work as stream is a borrowed type with lending iterator implements.
    //! // tokio::spawn(async move {
    //! //    let stream = stream;
    //! // })
    //!
    //! // in order to remove lifetime constraint we can do following:
    //!
    //! // convert borrowed stream to owned stream where lifetime constraints are lifted.
    //! let mut stream = xitca_postgres::RowStreamOwned::from(stream);
    //!
    //! // spawn async task and move stream into it.
    //! tokio::spawn(async move {
    //!     // use async iterator to handle rows.
    //!     use futures::stream::TryStreamExt;
    //!     while let Some(row) = stream.try_next().await? {
    //!         // handle row
    //!     }
    //!     Ok::<_, Error>(())
    //! });
    //! # Ok(())
    //! # }
    //! ```
    //!
    //! [`futures::Stream`]: futures_core::stream::Stream

    pub use crate::statement::compat::StatementGuarded;
}

pub mod dev {
    //! traits for extending functionalities through external crate

    pub use crate::client::ClientBorrowMut;
    pub use crate::copy::r#Copy;
    pub use crate::driver::codec::{encode::Encode, response::IntoResponse, Response};
    pub use crate::prepare::Prepare;
    pub use crate::query::Query;
}

use core::{future::Future, pin::Pin, sync::atomic::AtomicUsize};

use xitca_io::io::AsyncIo;

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

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
    /// Connect to database, returning [Client] and [Driver] on success
    ///
    /// # Examples:
    /// ```rust
    /// use std::future::IntoFuture;
    /// use xitca_postgres::{Execute, Postgres};
    ///
    /// # async fn connect() {
    /// let cfg = String::from("postgres://user:pass@localhost/db");
    /// let (client, driver) = Postgres::new(cfg).connect().await.unwrap();
    ///
    /// // spawn driver as async task.
    /// tokio::spawn(driver.into_future());
    ///
    /// // use client for query.
    /// "SELECT 1".execute(&client).await.unwrap();
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
    _assert_send(Postgres::new("postgres://postgres:postgres@localhost/postgres").connect());
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

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn config_error() {
        let mut cfg = Config::new();

        cfg.dbname("postgres").user("postgres").password("postgres");

        let mut cfg1 = cfg.clone();
        cfg1.host("localhost");
        Postgres::new(cfg1).connect().await.err().unwrap();

        cfg.port(5432);
        Postgres::new(cfg).connect().await.err().unwrap();
    }
}
