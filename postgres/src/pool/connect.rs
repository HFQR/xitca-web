use core::future::Future;

use xitca_io::io::AsyncIo;

use crate::{
    BoxedFuture, Postgres,
    client::Client,
    config::Config,
    driver::{Driver, generic::GenericDriver},
    error::Error,
    iter::AsyncLendingIterator,
};

/// trait for how to produce a new [`Client`] and push it into connection pool
pub trait Connect: Send + Sync {
    fn connect(&self, cfg: Config) -> impl Future<Output = Result<Client, Error>> + Send;
}

pub(super) trait ConnectorDyn: Send + Sync {
    fn connect_dyn(&self, cfg: Config) -> BoxedFuture<'_, Result<Client, Error>>;
}

impl<C> ConnectorDyn for C
where
    C: Connect + Send + Sync,
{
    fn connect_dyn(&self, cfg: Config) -> BoxedFuture<'_, Result<Client, Error>> {
        Box::pin(self.connect(cfg))
    }
}

pub(super) struct DefaultConnector;

impl Connect for DefaultConnector {
    async fn connect(&self, cfg: Config) -> Result<Client, Error> {
        let (client, driver) = Postgres::new(cfg).connect().await?;
        match driver {
            Driver::Tcp(drv) => {
                #[cfg(feature = "io-uring")]
                {
                    drive_uring(drv)
                }

                #[cfg(not(feature = "io-uring"))]
                {
                    drive(drv)
                }
            }
            Driver::Dynamic(drv) => drive(drv),
            #[cfg(feature = "tls")]
            Driver::Tls(drv) => drive(drv),
            #[cfg(unix)]
            Driver::Unix(drv) => drive(drv),
            #[cfg(all(unix, feature = "tls"))]
            Driver::UnixTls(drv) => drive(drv),
            #[cfg(feature = "quic")]
            Driver::Quic(drv) => drive(drv),
        };
        Ok(client)
    }
}

fn drive(mut drv: GenericDriver<impl AsyncIo + Send + 'static>) {
    tokio::task::spawn(async move {
        while drv.try_next().await?.is_some() {
            // TODO: add notify listen callback to Pool
        }
        Ok::<_, Error>(())
    });
}

#[cfg(feature = "io-uring")]
fn drive_uring(drv: GenericDriver<xitca_io::net::TcpStream>) {
    use core::{async_iter::AsyncIterator, future::poll_fn, pin::pin};

    tokio::task::spawn_local(async move {
        let mut iter = pin!(crate::driver::io_uring::UringDriver::from_tcp(drv).into_iter());
        while let Some(res) = poll_fn(|cx| iter.as_mut().poll_next(cx)).await {
            let _ = res?;
        }
        Ok::<_, Error>(())
    });
}
