pub(crate) mod codec;
pub(crate) mod io;

#[cfg(feature = "tls")]
mod tls;

#[cfg(not(feature = "quic"))]
mod raw;
#[cfg(not(feature = "quic"))]
pub(crate) use raw::*;

#[cfg(feature = "quic")]
mod quic;
#[cfg(feature = "quic")]
pub(crate) use quic::*;

use std::net::SocketAddr;

use xitca_service::AsyncClosure;

use super::{
    config::{Config, Host},
    error::Error,
};

#[cold]
#[inline(never)]
async fn try_connect_multi<F, O>(cfg: &Config, func: F) -> Result<O, Error>
where
    F: for<'f> AsyncClosure<(&'f Host, &'f Config), Output = Result<O, Error>>,
{
    let mut err = None;

    for host in cfg.get_hosts() {
        match func.call((host, cfg)).await {
            Ok(t) => return Ok(t),
            Err(e) => err = Some(e),
        }
    }

    Err(err.unwrap())
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
