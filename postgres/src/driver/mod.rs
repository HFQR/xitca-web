pub(crate) mod codec;
pub(crate) mod generic;

mod connect;
mod response;

pub use response::Response;

pub(crate) use generic::DriverTx;

#[cfg(feature = "tls")]
mod tls;

#[cfg(feature = "io-uring")]
mod io_uring;

#[cfg(feature = "quic")]
pub(crate) mod quic;

use core::{
    future::{Future, IntoFuture},
    pin::Pin,
};

use std::net::SocketAddr;

use postgres_protocol::message::backend;
use xitca_io::bytes::BytesMut;

use super::{client::Client, config::Config, error::Error, iter::AsyncLendingIterator};

use {self::generic::GenericDriver, xitca_io::net::TcpStream};

#[cfg(feature = "tls")]
use xitca_tls::rustls::{ClientConnection, TlsStream};

#[cfg(unix)]
use xitca_io::net::UnixStream;

pub(super) async fn connect(cfg: &mut Config) -> Result<(Client, Driver), Error> {
    let mut err = None;
    let hosts = cfg.get_hosts().to_vec();
    for host in hosts {
        match self::connect::_connect(host, cfg).await {
            Ok((tx, drv)) => return Ok((Client::new(tx), drv)),
            Err(e) => err = Some(e),
        }
    }

    Err(err.unwrap())
}

/// async driver of [Client](crate::Client).
/// it handles IO and emit server sent message that do not belong to any query with [AsyncLendingIterator]
/// trait impl.
///
/// # Examples:
/// ```rust
/// use std::future::IntoFuture;
/// use xitca_postgres::{AsyncLendingIterator, Driver};
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
pub struct Driver {
    inner: _Driver,
}

impl Driver {
    // run till the connection is closed by Client.
    async fn run_till_closed(self) {
        let _ = match self.inner {
            _Driver::Tcp(drv) => drv.run().await,
            #[cfg(feature = "tls")]
            _Driver::Tls(drv) => drv.run().await,
            #[cfg(unix)]
            _Driver::Unix(drv) => drv.run().await,
            #[cfg(all(unix, feature = "tls"))]
            _Driver::UnixTls(drv) => drv.run().await,
            #[cfg(feature = "quic")]
            _Driver::Quic(drv) => drv.run().await,
        };
    }
}

impl Driver {
    pub(super) fn tcp(drv: GenericDriver<TcpStream>) -> Self {
        Self {
            inner: _Driver::Tcp(drv),
        }
    }

    #[cfg(feature = "tls")]
    pub(super) fn tls(drv: GenericDriver<TlsStream<ClientConnection, TcpStream>>) -> Self {
        Self {
            inner: _Driver::Tls(drv),
        }
    }

    #[cfg(unix)]
    pub(super) fn unix(drv: GenericDriver<UnixStream>) -> Self {
        Self {
            inner: _Driver::Unix(drv),
        }
    }

    #[cfg(all(unix, feature = "tls"))]
    pub(super) fn unix_tls(drv: GenericDriver<TlsStream<ClientConnection, UnixStream>>) -> Self {
        Self {
            inner: _Driver::UnixTls(drv),
        }
    }

    #[cfg(feature = "quic")]
    pub(super) fn quic(drv: GenericDriver<crate::driver::quic::QuicStream>) -> Self {
        Self {
            inner: _Driver::Quic(drv),
        }
    }
}
// TODO: use Box<dyn AsyncIterator> when life time GAT is object safe.
enum _Driver {
    Tcp(GenericDriver<TcpStream>),
    #[cfg(feature = "tls")]
    Tls(GenericDriver<TlsStream<ClientConnection, TcpStream>>),
    #[cfg(unix)]
    Unix(GenericDriver<UnixStream>),
    #[cfg(all(unix, feature = "tls"))]
    UnixTls(GenericDriver<TlsStream<ClientConnection, UnixStream>>),
    #[cfg(feature = "quic")]
    Quic(GenericDriver<crate::driver::quic::QuicStream>),
}

impl AsyncLendingIterator for Driver {
    type Ok<'i> = backend::Message where Self: 'i;
    type Err = Error;

    #[inline]
    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        match self.inner {
            _Driver::Tcp(ref mut drv) => drv.try_next().await,
            #[cfg(feature = "tls")]
            _Driver::Tls(ref mut drv) => drv.try_next().await,
            #[cfg(unix)]
            _Driver::Unix(ref mut drv) => drv.try_next().await,
            #[cfg(all(unix, feature = "tls"))]
            _Driver::UnixTls(ref mut drv) => drv.try_next().await,
            #[cfg(feature = "quic")]
            _Driver::Quic(ref mut drv) => drv.try_next().await,
        }
    }
}

impl IntoFuture for Driver {
    type Output = ();
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run_till_closed())
    }
}

async fn resolve(host: &str, ports: &[u16]) -> Result<Vec<SocketAddr>, Error> {
    let addrs = tokio::net::lookup_host((host, 0))
        .await?
        .flat_map(|mut addr| {
            ports.iter().map(move |port| {
                addr.set_port(*port);
                addr
            })
        })
        .collect::<Vec<_>>();
    Ok(addrs)
}

#[cfg(feature = "io-uring")]
impl Driver {
    // downcast Driver to IoUringDriver if it's Tcp variant.
    // IoUringDriver can not be a new variant of Dirver as it's !Send.
    pub fn try_into_io_uring_tcp(self) -> io_uring::IoUringDriver<xitca_io::net::io_uring::TcpStream> {
        match self.inner {
            _Driver::Tcp(drv) => {
                let std = drv.io.into_std().unwrap();
                let tcp = xitca_io::net::io_uring::TcpStream::from_std(std);
                io_uring::IoUringDriver::new(
                    tcp,
                    drv.state.take_rx(),
                    drv.write_buf.into_inner(),
                    drv.read_buf.into_inner(),
                    drv.res,
                )
            }
            _ => todo!(),
        }
    }
}

pub(crate) trait Drive: Send {
    fn send(&mut self, msg: BytesMut) -> impl Future<Output = Result<(), Error>> + Send;

    fn recv(&mut self) -> impl Future<Output = Result<backend::Message, Error>> + Send;
}
