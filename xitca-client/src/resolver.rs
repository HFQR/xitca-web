use std::net::{SocketAddr, ToSocketAddrs};

use futures_core::future::BoxFuture;

use crate::connect::Connect;
use crate::error::Error;

pub(crate) enum Resolver {
    Default,
    Custom(Box<dyn Resolve>),
}

impl Default for Resolver {
    fn default() -> Self {
        Self::Default
    }
}

impl Resolver {
    pub(crate) fn custom(resolver: impl Resolve + 'static) -> Self {
        Self::Custom(Box::new(resolver))
    }

    pub(crate) async fn resolve(&self, connect: &mut Connect) -> Result<(), Error> {
        match *self {
            Self::Default => {
                let host = format!("{}:{}", connect.hostname(), connect.port());
                let addrs = tokio::task::spawn_blocking(move || host.to_socket_addrs())
                    .await
                    .unwrap()?;

                connect.set_addrs(addrs);
            }
            Self::Custom(ref resolve) => {
                let addrs = resolve.resolve(connect.hostname(), connect.port()).await?;
                connect.set_addrs(addrs);
            }
        };

        Ok(())
    }
}

pub trait Resolve: Send {
    fn resolve<'s, 'h, 'f>(&'s self, hostname: &'h str, port: u16) -> BoxFuture<'f, Result<Vec<SocketAddr>, Error>>
    where
        's: 'f,
        'h: 'f;
}
