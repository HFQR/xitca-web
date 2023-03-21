#[cfg(not(feature = "quic"))]
mod raw;
#[cfg(not(feature = "quic"))]
pub use raw::*;

#[cfg(feature = "quic")]
mod quic;
#[cfg(feature = "quic")]
pub use quic::*;

use xitca_service::AsyncClosure;

use super::{
    config::{Config, Host},
    error::Error,
};

#[cold]
#[inline(never)]
pub(crate) async fn try_connect_multi<F, O>(cfg: &Config, func: F) -> Result<O, Error>
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
