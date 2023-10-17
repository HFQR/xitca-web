use core::future::Future;

use std::net::{SocketAddr, ToSocketAddrs};

use futures_core::future::BoxFuture;

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
        let addrs = match *self {
            Self::Std => {
                let host = connect.hostname().to_string();
                let port = connect.port();
                tokio::task::spawn_blocking(move || (host, port).to_socket_addrs())
                    .await
                    .unwrap()?
            }
            Self::Custom(ref resolve) => resolve
                .resolve_dyn(connect.hostname(), connect.port())
                .await?
                .into_iter(),
        };

        connect.set_addrs(addrs);

        Ok(())
    }
}

/// Trait for custom DNS resolver.
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
///     // hostname is stripped of port number(if given).
///     async fn resolve(&self, hostname: &str, port: u16) -> Result<Vec<SocketAddr>, Error> {
///         // Your DNS resolve logic goes here.
///         Ok(vec![])
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
