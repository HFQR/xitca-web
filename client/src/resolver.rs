use core::future::Future;

use std::net::{SocketAddr, ToSocketAddrs};

use futures_core::future::BoxFuture;
use tokio::task;

use crate::{connect::Connect, error::Error};

pub(crate) enum Resolver {
    Std,
    Custom(Box<dyn ResolveDyn>),
}

impl Default for Resolver {
    fn default() -> Self {
        Self::Std
    }
}

impl Resolver {
    pub(crate) fn custom(resolver: impl Resolve + 'static) -> Self {
        Self::Custom(Box::new(resolver))
    }

    pub(crate) async fn resolve(&self, connect: &mut Connect<'_>) -> Result<(), Error> {
        match *self {
            Self::Std => {
                let host = connect.hostname().to_string();

                let task = if connect
                    .hostname()
                    .splitn(2, ':')
                    .last()
                    .and_then(|p| p.parse::<u16>().ok())
                    .is_some()
                {
                    task::spawn_blocking(move || host.to_socket_addrs())
                } else {
                    let host = (host, connect.port());
                    task::spawn_blocking(move || host.to_socket_addrs())
                };

                let addrs = task.await.unwrap()?;

                connect.set_addrs(addrs);
            }
            Self::Custom(ref resolve) => {
                let addrs = resolve.resolve_dyn(connect.hostname(), connect.port()).await?;
                connect.set_addrs(addrs);
            }
        };

        Ok(())
    }
}

/// Trait for custom resolver.
///
/// # Examples
/// ```rust
/// use std::net::SocketAddr;
///
/// use xitca_client::{error::Error, ClientBuilder, Resolve};
///
/// struct MyResolver;
///
/// impl Resolve for MyResolver {
///     async fn resolve(&self, hostname: &str, port: u16) -> Result<Vec<SocketAddr>, Error> {
///         // Your DNS resolve logic goes here.
///         todo!()
///     }
/// }
///
/// # fn resolve() {
/// let client = ClientBuilder::new().resolver(MyResolver).finish();
/// # }
/// ```
pub trait Resolve: Send + Sync {
    /// *. hostname does not include port number.
    fn resolve(&self, hostname: &str, port: u16) -> impl Future<Output = Result<Vec<SocketAddr>, Error>> + Send;
}

pub(crate) trait ResolveDyn: Send + Sync {
    fn resolve_dyn<'s, 'h>(&'s self, hostname: &'h str, port: u16) -> BoxFuture<'h, Result<Vec<SocketAddr>, Error>>
    where
        's: 'h;
}

impl<R> ResolveDyn for R
where
    R: Resolve,
{
    #[inline]
    fn resolve_dyn<'s, 'h>(&'s self, hostname: &'h str, port: u16) -> BoxFuture<'h, Result<Vec<SocketAddr>, Error>>
    where
        's: 'h,
    {
        Box::pin(self.resolve(hostname, port))
    }
}
