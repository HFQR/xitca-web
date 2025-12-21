//! client driver module.

pub(crate) mod codec;
pub(crate) mod generic;

mod connect;

pub(crate) use generic::DriverTx;

#[cfg(feature = "tls")]
mod tls;

#[cfg(feature = "quic")]
pub(crate) mod quic;

#[cfg(feature = "io-uring")]
pub(crate) mod io_uring;

use core::{
    future::{Future, IntoFuture},
    net::SocketAddr,
    pin::Pin,
};

use std::io;

use postgres_protocol::message::{backend, frontend};
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, AsyncIoDyn, Interest},
    net::TcpStream,
};

use super::{
    client::Client,
    config::{Config, SslMode, SslNegotiation},
    error::{ConfigError, Error, unexpected_eof_err},
    iter::AsyncLendingIterator,
    session::{ConnectInfo, Session},
};

use self::generic::GenericDriver;

#[cfg(feature = "tls")]
use xitca_tls::rustls::{ClientConnection, TlsStream};

#[cfg(unix)]
use xitca_io::net::UnixStream;

pub(super) async fn connect(cfg: &mut Config) -> Result<(Client, Driver), Error> {
    if cfg.get_hosts().is_empty() {
        return Err(ConfigError::EmptyHost.into());
    }

    if cfg.get_ports().is_empty() {
        return Err(ConfigError::EmptyPort.into());
    }

    let mut err = None;
    let hosts = cfg.get_hosts().to_vec();
    for host in hosts {
        match self::connect::connect_host(host, cfg).await {
            Ok((tx, session, drv)) => return Ok((Client::new(tx, session), drv)),
            Err(e) => err = Some(e),
        }
    }

    Err(err.unwrap())
}

pub(super) async fn connect_io<Io>(io: Io, cfg: &mut Config) -> Result<(Client, Driver), Error>
where
    Io: AsyncIo + Send + 'static,
{
    let (tx, session, drv) = prepare_driver(ConnectInfo::default(), Box::new(io) as _, cfg).await?;
    Ok((Client::new(tx, session), Driver::Dynamic(drv)))
}

pub(super) async fn connect_info(info: ConnectInfo) -> Result<(DriverTx, Driver), Error> {
    self::connect::connect_info(info).await
}

async fn prepare_driver<Io>(
    info: ConnectInfo,
    io: Io,
    cfg: &mut Config,
) -> Result<(DriverTx, Session, GenericDriver<Io>), Error>
where
    Io: AsyncIo + Send + 'static,
{
    let (mut drv, tx) = GenericDriver::new(io);
    let session = Session::prepare_session(info, &mut drv, cfg).await?;
    Ok((tx, session, drv))
}

async fn should_connect_tls<Io>(io: &mut Io, ssl_mode: SslMode, ssl_negotiation: SslNegotiation) -> Result<bool, Error>
where
    Io: AsyncIo,
{
    async fn query_tls_availability<Io>(io: &mut Io) -> std::io::Result<bool>
    where
        Io: AsyncIo,
    {
        let mut buf = BytesMut::new();
        frontend::ssl_request(&mut buf);

        while !buf.is_empty() {
            match io.write(&buf) {
                Ok(0) => return Err(unexpected_eof_err()),
                Ok(n) => buf.advance(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    io.ready(Interest::WRITABLE).await?;
                }
                Err(e) => return Err(e),
            }
        }

        let mut buf = [0];
        loop {
            match io.read(&mut buf) {
                Ok(0) => return Err(unexpected_eof_err()),
                Ok(_) => return Ok(buf[0] == b'S'),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    io.ready(Interest::READABLE).await?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    match ssl_mode {
        SslMode::Disable => Ok(false),
        _ if matches!(ssl_negotiation, SslNegotiation::Direct) => Ok(true),
        mode => match (query_tls_availability(io).await?, mode) {
            (false, SslMode::Require) => Err(Error::todo()),
            (bool, _) => Ok(bool),
        },
    }
}

async fn dns_resolve<'p>(host: &'p str, ports: &'p [u16]) -> Result<impl Iterator<Item = SocketAddr> + 'p, Error> {
    let addrs = tokio::net::lookup_host((host, 0)).await?.flat_map(|mut addr| {
        ports.iter().map(move |port| {
            addr.set_port(*port);
            addr
        })
    });
    Ok(addrs)
}

/// async driver of [`Client`]
///
/// it handles IO and emit server sent message that do not belong to any query with [`AsyncLendingIterator`]
/// trait impl.
///
/// # Examples
/// ```
/// use std::future::IntoFuture;
/// use xitca_postgres::{iter::AsyncLendingIterator, Driver};
///
/// // drive the client and listen to server notify at the same time.
/// fn drive_with_server_notify(mut drv: Driver) {
///     tokio::spawn(async move {
///         while let Ok(Some(msg)) = drv.try_next().await {
///             // *Note:
///             // handle message must be non-blocking to prevent starvation of driver.
///         }
///     });
/// }
///
/// // drive client without handling notify.
/// fn drive_only(drv: Driver) {
///     tokio::spawn(drv.into_future());
/// }
/// ```
///
/// # Lifetime
/// Driver and [`Client`] have a dependent lifetime where either side can trigger the other part to shutdown.
/// From Driver side it's in the form of dropping ownership.
/// ## Examples
/// ```
/// # use xitca_postgres::{error::Error, Config, Execute, Postgres};
/// # async fn shut_down(cfg: Config) -> Result<(), Error> {
/// // connect to a database
/// let (cli, drv) = Postgres::new(cfg).connect().await?;
///
/// // drop driver
/// drop(drv);
///
/// // client will always return error when it's driver is gone.
/// let e = "SELECT 1".query(&cli).await.unwrap_err();
/// // a shortcut method can be used to determine if the error is caused by a shutdown driver.
/// assert!(e.is_driver_down());
///
/// # Ok(())
/// # }
/// ```
///
// TODO: use Box<dyn AsyncIterator> when life time GAT is object safe.
pub enum Driver {
    Tcp(GenericDriver<TcpStream>),
    Dynamic(GenericDriver<Box<dyn AsyncIoDyn + Send>>),
    #[cfg(feature = "tls")]
    Tls(GenericDriver<TlsStream<ClientConnection, TcpStream>>),
    #[cfg(unix)]
    Unix(GenericDriver<UnixStream>),
    #[cfg(all(unix, feature = "tls"))]
    UnixTls(GenericDriver<TlsStream<ClientConnection, UnixStream>>),
    #[cfg(feature = "quic")]
    Quic(GenericDriver<crate::driver::quic::QuicStream>),
}

impl Driver {
    #[inline]
    pub(crate) async fn send(&mut self, buf: BytesMut) -> Result<(), Error> {
        match self {
            Self::Tcp(drv) => drv.send(buf).await,
            Self::Dynamic(drv) => drv.send(buf).await,
            #[cfg(feature = "tls")]
            Self::Tls(drv) => drv.send(buf).await,
            #[cfg(unix)]
            Self::Unix(drv) => drv.send(buf).await,
            #[cfg(all(unix, feature = "tls"))]
            Self::UnixTls(drv) => drv.send(buf).await,
            #[cfg(feature = "quic")]
            Self::Quic(drv) => drv.send(buf).await,
        }
    }

    // try to unwrap driver that using unencrypted tcp connection
    pub fn try_into_tcp(self) -> Option<GenericDriver<TcpStream>> {
        match self {
            Self::Tcp(drv) => Some(drv),
            _ => None,
        }
    }

    #[cfg(feature = "io-uring")]
    pub fn try_into_uring(self) -> Option<io_uring::UringDriver> {
        self.try_into_tcp().map(io_uring::UringDriver::from_tcp)
    }
}

impl AsyncLendingIterator for Driver {
    type Ok<'i>
        = backend::Message
    where
        Self: 'i;
    type Err = Error;

    #[inline]
    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        match self {
            Self::Tcp(drv) => drv.try_next().await,
            Self::Dynamic(drv) => drv.try_next().await,
            #[cfg(feature = "tls")]
            Self::Tls(drv) => drv.try_next().await,
            #[cfg(unix)]
            Self::Unix(drv) => drv.try_next().await,
            #[cfg(all(unix, feature = "tls"))]
            Self::UnixTls(drv) => drv.try_next().await,
            #[cfg(feature = "quic")]
            Self::Quic(drv) => drv.try_next().await,
        }
    }
}

impl IntoFuture for Driver {
    type Output = Result<(), Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(mut self) -> Self::IntoFuture {
        Box::pin(async move {
            while self.try_next().await?.is_some() {}
            Ok(())
        })
    }
}
